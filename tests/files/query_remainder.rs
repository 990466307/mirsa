#![feature(rustc_private)]

extern crate rustc_driver;
extern crate rustc_interface;
extern crate rustc_middle;

use mirsa_analysis::interprocedural::AnalysisSession;
use mirsa_analysis::query::{Bindings, QueryConfig, Verdict};
use mirsa_framework::forward::PathForwardAnalysisConfig;
use rustc_driver::{Callbacks, Compilation};
use rustc_middle::mir::{Location, TerminatorKind};
use std::time::Duration;

struct Checks;
impl Callbacks for Checks {
    fn after_analysis<'tcx>(
        &mut self,
        _: &rustc_interface::interface::Compiler,
        tcx: rustc_middle::ty::TyCtxt<'tcx>,
    ) -> Compilation {
        let mut checked = 0;
        for id in tcx.mir_keys(()) {
            let name = tcx.item_name(id.to_def_id()).to_string();
            if !name.starts_with("check_") {
                continue;
            }
            let body = tcx.optimized_mir(id.to_def_id());
            let session = AnalysisSession::new(PathForwardAnalysisConfig {
                max_paths: 8,
                widen_after_iterations: Some(8),
                max_duration: Some(Duration::from_secs(10)),
            });
            let result = session.analyze(tcx, body);
            assert!(result.completed);
            // This is a transfer/reduction guarantee: SMT must not mask an
            // interval-domain omission, including self-assignments.
            let view = mirsa_analysis::query::AnalysisView::new(tcx, body, &result).with_config(
                QueryConfig {
                    use_solver: false,
                    ..Default::default()
                },
            );
            for (bb, data) in body.basic_blocks.iter_enumerated() {
                let TerminatorKind::Call { func, args, .. } = &data.terminator().kind else {
                    continue;
                };
                let rustc_middle::ty::TyKind::FnDef(callee, _) =
                    func.ty(&body.local_decls, tcx).kind()
                else {
                    continue;
                };
                if tcx.item_name(*callee).as_str() != "observe" {
                    continue;
                }
                let at = Location {
                    block: bb,
                    statement_index: data.statements.len(),
                };
                let bindings = Bindings::new().value("r", args[0].node.clone());
                let property = match name.as_str() {
                    "check_positive" => "0 <= math($r) && math($r) < 100",
                    "check_negative" => "-100 < math($r) && math($r) <= 0",
                    _ => "-100 < math($r) && math($r) < 100",
                };
                let proof = view.check(property, at, &bindings).unwrap();
                assert!(!proof.unreachable, "{name}: {proof:?}");
                assert_eq!(proof.verdict, Verdict::T, "{name}: {proof:?}");
                let wrong = view.check("math($r) == 100", at, &bindings).unwrap();
                assert_eq!(wrong.verdict, Verdict::F, "{name}: {wrong:?}");
                assert!(wrong.definite_violation);
                checked += 1;
            }
        }
        assert_eq!(checked, 3);
        Compilation::Stop
    }
}

#[test]
fn remainder_transfer_retains_bounds_after_self_assignment() {
    let source = std::env::temp_dir().join(format!("mirsa-remainder-{}.rs", std::process::id()));
    std::fs::write(
        &source,
        r#"
        unsafe extern "C" { fn observe(r: i64); }
        pub unsafe fn check_positive(mut x: i64) {
            if 100 <= x && x <= 999 { x %= 100; observe(x); }
        }
        pub unsafe fn check_negative(mut x: i64) {
            if -999 <= x && x <= -100 { x %= -100; observe(x); }
        }
        pub unsafe fn check_variable(mut x: i64, divisor: i64) {
            if -100 <= divisor && divisor <= 100 && divisor != 0 {
                x %= divisor;
                observe(x);
            }
        }
    "#,
    )
    .unwrap();
    rustc_driver::run_compiler(
        &[
            "rustc".into(),
            source.to_string_lossy().into_owned(),
            "--edition=2021".into(),
            "--crate-name=mirsa_remainder".into(),
            "--crate-type=lib".into(),
            "--emit=metadata".into(),
            "-Zmir-opt-level=0".into(),
        ],
        &mut Checks,
    );
    std::fs::remove_file(source).unwrap();
}
