#![feature(rustc_private)]
extern crate rustc_driver;
extern crate rustc_interface;
extern crate rustc_middle;

use mirsa_analysis::interprocedural::AnalysisSession;
use mirsa_analysis::query::{AnalysisView, Bindings, Verdict};
use mirsa_framework::forward::PathForwardAnalysisConfig;
use rustc_driver::{Callbacks, Compilation};
use rustc_middle::mir::{Location, Operand, TerminatorKind};
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
            let name = tcx.item_name(id.to_def_id());
            let (property, expected) = match name.as_str() {
                "inclusive_loop" => (
                    "0 <= offset($v) && offset($v) + 4 <= extent($v)",
                    Some(Verdict::F),
                ),
                "dead_tail" => ("$v <= 3", Some(Verdict::T)),
                "alias_write" | "unknown_write" => ("$v == 3", Some(Verdict::F)),
                "loop_write" => ("!is_null($v)", Some(Verdict::F)),
                "loop_history" => ("$v == 2", Some(Verdict::F)),
                "guarded" => ("$v < 8", Some(Verdict::T)),
                _ => continue,
            };
            let body = tcx.optimized_mir(id.to_def_id());
            let config = PathForwardAnalysisConfig {
                max_paths: 8,
                widen_after_iterations: Some(8),
                max_duration: Some(Duration::from_secs(10)),
            };
            let full_session = AnalysisSession::new(config);
            let full = full_session.analyze(tcx, body);
            assert!(full.completed);
            for (bb, data) in body.basic_blocks.iter_enumerated() {
                let TerminatorKind::Call { func, args, .. } = &data.terminator().kind else {
                    continue;
                };
                let rustc_middle::ty::TyKind::FnDef(callee, _) =
                    func.ty(&body.local_decls, tcx).kind()
                else {
                    continue;
                };
                if !tcx.item_name(*callee).as_str().starts_with("observe") {
                    continue;
                }
                let at = Location {
                    block: bb,
                    statement_index: data.statements.len(),
                };
                let bindings = Bindings::new()
                    .location("point", at)
                    .value("v", args[0].node.clone());
                let full_view = AnalysisView::new(tcx, body, &full);
                let baseline = full_view.check(property, at, &bindings).unwrap();
                let session = AnalysisSession::new(config);
                let view = session.analyze_for_queries(tcx, body);
                let sliced = view.check(property, at, &bindings).unwrap();
                assert_eq!(
                    sliced.verdict, baseline.verdict,
                    "{name}: {sliced:?} vs {baseline:?}"
                );
                if let Some(expected) = expected {
                    assert_eq!(sliced.verdict, expected, "{name}: {sliced:?}");
                }
                if name.as_str() == "loop_history" {
                    for proof in [
                        full_view.check("$v < 3", at, &bindings).unwrap(),
                        view.check("$v < 3", at, &bindings).unwrap(),
                    ] {
                        assert_eq!(proof.verdict, Verdict::T, "{proof:?}");
                        assert!(!proof.unreachable);
                    }
                }
                let stats = session.slice_stats().unwrap();
                if name.as_str() == "dead_tail" {
                    assert!(stats.retained_blocks < stats.total_blocks, "{stats:?}");
                    assert!(
                        stats.retained_statements < stats.total_statements,
                        "{stats:?}"
                    );
                }
                // A complete facts request must still use the complete analysis,
                // including after this same view has served a sliced query.
                let facts = "facts($v) at 'point";
                let actual = view.query(facts, &bindings).unwrap();
                let original = full_view.query(facts, &bindings).unwrap();
                assert_eq!(
                    serde_json::to_value(actual).unwrap(),
                    serde_json::to_value(original).unwrap(),
                    "{name}"
                );
                assert!(session.slice_stats().is_none());
                assert!(matches!(args[0].node, Operand::Copy(_) | Operand::Move(_)));
                checked += 1;
            }
        }
        assert_eq!(checked, 7);
        Compilation::Stop
    }
}

#[test]
fn query_slice_keeps_controls_alias_effects_and_loop_dependencies() {
    let source = std::env::temp_dir().join(format!("mirsa-slice-{}.rs", std::process::id()));
    std::fs::write(
        &source,
        r#"
unsafe extern "C" {
    fn observe(v: usize);
    fn observe_ptr(v: *const u8);
    fn mutate(v: *mut usize);
}
unsafe fn observe_inbound(_: *const u32) {}
pub unsafe fn inclusive_loop() {
    let arr = [10u32, 20, 30, 40];
    let ptr = arr.as_ptr();
    for i in 0..=arr.len() { observe_inbound(ptr.offset(i as isize)); }
}
pub unsafe fn dead_tail(n: usize) {
    let unrelated = n ^ 5;
    let v = n & 3;
    observe(v);
    if unrelated == 123 { mutate(unrelated as *mut usize); }
}
pub unsafe fn alias_write() {
    let mut v = 3;
    let p = &mut v as *mut usize;
    *p = 9;
    observe(v);
}
pub unsafe fn unknown_write() {
    let mut v = 3;
    mutate(&mut v);
    observe(v);
}
pub unsafe fn loop_write(n: usize) {
    let mut p = 1usize as *const u8;
    let mut i = 0;
    while i < n { if i == 12 { p = 0usize as *const u8; } i += 1; }
    observe_ptr(p);
}
pub unsafe fn loop_history() {
    let mut i = 0usize;
    while i < 3 { observe(i); i += 1; }
}
pub unsafe fn guarded(v: usize) { if v < 8 { observe(v); } }
"#,
    )
    .unwrap();
    for opt in [0, 1] {
        rustc_driver::run_compiler(
            &[
                "rustc".into(),
                source.to_string_lossy().into_owned(),
                "--edition=2021".into(),
                "--crate-name=mirsa_slice".into(),
                "--crate-type=lib".into(),
                "--emit=metadata".into(),
                format!("-Zmir-opt-level={opt}"),
            ],
            &mut Checks,
        );
    }
    std::fs::remove_file(source).unwrap();
}
