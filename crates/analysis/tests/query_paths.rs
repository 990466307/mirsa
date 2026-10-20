#![feature(rustc_private)]

extern crate rustc_driver;
extern crate rustc_interface;
extern crate rustc_middle;

use mirsa_analysis::interprocedural::AnalysisSession;
use mirsa_analysis::query::{Bindings, Output, QueryConfig, Verdict};
use mirsa_framework::forward::PathForwardAnalysisConfig;
use rustc_driver::{Callbacks, Compilation};
use rustc_middle::mir::{Location, TerminatorKind};
use std::time::Duration;

struct QueryChecks;

impl Callbacks for QueryChecks {
    fn after_analysis<'tcx>(
        &mut self,
        _: &rustc_interface::interface::Compiler,
        tcx: rustc_middle::ty::TyCtxt<'tcx>,
    ) -> Compilation {
        let mut checked = 0;
        for id in tcx.mir_keys(()) {
            let expected = match tcx.item_name(id.to_def_id()).as_str() {
                "mixed" | "false_and_unknown" => Verdict::F,
                "safe" => Verdict::T,
                "unknown" => Verdict::F,
                _ => continue,
            };
            let body = tcx.optimized_mir(id.to_def_id());
            let session = AnalysisSession::new(PathForwardAnalysisConfig {
                max_paths: 8,
                widen_after_iterations: Some(8),
                max_duration: Some(Duration::from_secs(10)),
            });
            let result = session.analyze(tcx, body);
            assert!(result.completed);
            for (bb, data) in body.basic_blocks.iter_enumerated() {
                let TerminatorKind::Call { args, .. } = &data.terminator().kind else {
                    continue;
                };
                let location = Location {
                    block: bb,
                    statement_index: data.statements.len(),
                };
                let bindings = Bindings::new()
                    .location("call", location)
                    .value("p", args[0].node.clone());
                for use_solver in [false, cfg!(feature = "z3")] {
                    let view = mirsa_analysis::query::AnalysisView::new(tcx, body, &result)
                        .with_config(QueryConfig {
                            use_solver,
                            ..Default::default()
                        });
                    let report = view.query("!is_null($p) at 'call", &bindings).unwrap();
                    let Output::Check(check) = &report.output else {
                        panic!()
                    };
                    assert_eq!(check.result, expected == Verdict::T);
                    let proof = view.check("!is_null($p)", location, &bindings).unwrap();
                    assert_eq!(
                        proof.verdict,
                        expected,
                        "{}: {proof:?}",
                        tcx.item_name(id.to_def_id())
                    );
                    // The query must carry the same evidence as `check`, not just a
                    // boolean: `definite_violation` is what separates "refuted" from
                    // "unknown" for a caller that only sees the JSON.
                    let json = serde_json::to_value(&report).unwrap();
                    let detail = &json["output"]["result"];
                    assert_eq!(
                        detail["verdict"],
                        serde_json::to_value(proof.verdict).unwrap()
                    );
                    assert_eq!(
                        detail["definite_violation"],
                        serde_json::Value::Bool(proof.definite_violation)
                    );
                    assert_eq!(
                        detail["unreachable"],
                        serde_json::Value::Bool(proof.unreachable)
                    );
                    assert_eq!(
                        detail["paths"].as_array().map(Vec::len),
                        Some(proof.paths.len())
                    );
                    assert_eq!(format!("{report}"), format!("at 'call:\n{}", check.result));
                    assert!(!proof.unreachable);
                    assert_eq!(
                        proof.definite_violation,
                        matches!(
                            tcx.item_name(id.to_def_id()).as_str(),
                            "mixed" | "false_and_unknown"
                        )
                    );
                    assert_eq!(
                        proof.reason.is_some(),
                        tcx.item_name(id.to_def_id()).as_str() == "unknown"
                    );
                    if tcx.item_name(id.to_def_id()).as_str() == "mixed" {
                        assert!(proof.paths.iter().any(|p| p.verdict == Verdict::T));
                        assert!(proof.paths.iter().any(|p| p.verdict == Verdict::F));
                    }
                    let facts_report = view.query("facts($p) at 'call", &bindings).unwrap();
                    let Output::Facts(facts) = &facts_report.output else {
                        panic!()
                    };
                    assert!(
                        serde_json::to_value(&facts_report)
                            .unwrap()
                            .get("location")
                            .is_none()
                    );
                    assert!(format!("{facts_report:?}").starts_with("Report { output:"));
                    assert!(format!("{facts_report}").starts_with("at 'call:"));
                    let json = serde_json::to_value(&facts).unwrap();
                    for field in [
                        "expression",
                        "paths",
                        "path_id",
                        "merged",
                        "bindings",
                        "value",
                        "observations",
                        "stored",
                        "relations",
                    ] {
                        assert!(json.get(field).is_none());
                    }
                    if tcx.item_name(id.to_def_id()).as_str() == "mixed" {
                        let mirsa_analysis::query::FactValue::Pointer { may_be_null, .. } = &facts
                            .abstract_info
                            .iter()
                            .find(|f| {
                                f.subject == mirsa_analysis::query::Expression::variable("$p")
                            })
                            .unwrap()
                            .value
                        else {
                            panic!("{:?}", facts);
                        };
                        assert!(*may_be_null);
                    }
                }
                checked += 1;
            }
        }
        assert_eq!(checked, 4);
        Compilation::Stop
    }
}

#[test]
fn definite_failure_on_one_mir_path_is_false() {
    let source = std::env::temp_dir().join(format!("mirsa-query-paths-{}.rs", std::process::id()));
    std::fs::write(
        &source,
        r#"
        unsafe extern "C" { fn observe(p: *const u8); }
        pub unsafe fn mixed(flag: bool) {
            let p = if flag { 0usize as *const u8 } else { 1usize as *const u8 };
            observe(p);
        }
        pub unsafe fn false_and_unknown(flag: bool, other: *const u8) {
            let p = if flag { 0usize as *const u8 } else { other };
            observe(p);
        }
        pub unsafe fn safe(flag: bool) {
            let p = if flag { 1usize as *const u8 } else { 2usize as *const u8 };
            observe(p);
        }
        pub unsafe fn unknown(p: *const u8) { observe(p); }
    "#,
    )
    .unwrap();
    rustc_driver::run_compiler(
        &[
            "rustc".into(),
            source.to_string_lossy().into_owned(),
            "--edition=2021".into(),
            "--crate-name=mirsa_query_paths".into(),
            "--crate-type=lib".into(),
            "--emit=metadata".into(),
            "-Zmir-opt-level=0".into(),
        ],
        &mut QueryChecks,
    );
    std::fs::remove_file(source).unwrap();
}
