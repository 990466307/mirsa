#![feature(rustc_private)]
#![cfg(feature = "z3")]

extern crate rustc_driver;
extern crate rustc_hir;
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
            if !matches!(tcx.def_kind(*id), rustc_hir::def::DefKind::Fn) {
                continue;
            }
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
            // Check the independent guarantees of references: address validity
            // does not imply the presence of a nonzero allocation.
            let view = mirsa_analysis::query::AnalysisView::new(tcx, body, &result).with_config(
                QueryConfig {
                    solver_timeout: Duration::from_secs(1),
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
                for property in ["!is_null($r)", "address($r) % 4 == 0"] {
                    let proof = view.check(property, at, &bindings).unwrap();
                    assert!(!proof.unreachable, "{name}: {proof:?}");
                    assert_eq!(proof.verdict, Verdict::T, "{name} {property}: {proof:?}");
                }
                let live = view.check("is_live($r)", at, &bindings).unwrap();
                assert_eq!(
                    live.verdict,
                    if matches!(name.as_str(), "check_nonempty" | "check_sized") {
                        Verdict::T
                    } else {
                        Verdict::F
                    },
                    "{name}: {live:?}"
                );
                assert_eq!(live.definite_violation, name == "check_freed");
                assert_eq!(
                    live.reason.is_some(),
                    !matches!(
                        name.as_str(),
                        "check_nonempty" | "check_sized" | "check_freed"
                    )
                );
                checked += 1;
            }
        }
        assert_eq!(checked, 6);
        Compilation::Stop
    }
}

#[test]
fn references_require_an_allocation_only_for_nonzero_referents() {
    let source = std::env::temp_dir().join(format!(
        "mirsa-reference-validity-{}.rs",
        std::process::id()
    ));
    std::fs::write(
        &source,
        r#"
        unsafe extern "C" { fn observe(r: *const u32); }
        pub unsafe fn check_unknown(p: &[u32]) { observe(p.as_ptr()); }
        pub unsafe fn check_empty(p: &[u32]) { if p.is_empty() { observe(p.as_ptr()); } }
        pub unsafe fn check_nonempty(p: &[u32]) { if !p.is_empty() { observe(p.as_ptr()); } }
        pub unsafe fn check_sized(p: &u32) { observe(p as *const u32); }
        pub unsafe fn check_zst(p: &[u32; 0]) { observe(p as *const [u32; 0] as *const u32); }
        pub unsafe fn check_freed() {
            let layout = std::alloc::Layout::new::<u32>();
            let raw = std::alloc::alloc(layout);
            if raw.is_null() { return; }
            let slice = std::slice::from_raw_parts(raw as *const u32, 1);
            let p = slice.as_ptr();
            std::alloc::dealloc(raw, layout);
            observe(p);
        }
    "#,
    )
    .unwrap();
    rustc_driver::run_compiler(
        &[
            "rustc".into(),
            source.to_string_lossy().into_owned(),
            "--edition=2021".into(),
            "--crate-name=mirsa_reference_validity".into(),
            "--crate-type=lib".into(),
            "--emit=metadata".into(),
            "-Zmir-opt-level=0".into(),
        ],
        &mut Checks,
    );
    std::fs::remove_file(source).unwrap();
}
