#![feature(rustc_private)]
#![cfg(feature = "z3")]

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
                max_paths: 1,
                widen_after_iterations: Some(8),
                max_duration: Some(Duration::from_secs(10)),
            });
            let result = session.analyze(tcx, body);
            assert!(result.completed);
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
                let bindings = Bindings::new()
                    .value("p", args[0].node.clone())
                    .value("n", args[1].node.clone());
                let property = match name.as_str() {
                    "check_wrapping" => "address($p) == math($n + 12usize)",
                    "check_wrapping_sub" => "address($p) == math($n - 12usize)",
                    _ => "address($p) == math($n)",
                };
                let proof = view.check(property, at, &bindings).unwrap();
                assert!(!proof.unreachable, "{name}: {proof:?}");
                if matches!(name.as_str(), "check_overwrite" | "check_merge") {
                    assert_ne!(proof.verdict, Verdict::T, "{name}: {proof:?}");
                } else {
                    assert_eq!(proof.verdict, Verdict::T, "{name}: {proof:?}");
                }
                if name == "check_raw" || name == "check_unsigned" {
                    let live = view.check("is_live($p)", at, &bindings).unwrap();
                    assert_eq!(live.verdict, Verdict::F, "{name}: {live:?}");
                    assert!(!live.definite_violation);
                    assert!(live.reason.is_some());
                }
                checked += 1;
            }
        }
        assert_eq!(checked, 10);
        Compilation::Stop
    }
}

#[test]
fn pointer_casts_preserve_addresses_without_inventing_validity() {
    let source =
        std::env::temp_dir().join(format!("mirsa-pointer-casts-{}.rs", std::process::id()));
    std::fs::write(
        &source,
        r#"
        unsafe extern "C" { fn observe(p: *const u32, n: usize); }
        pub unsafe fn check_truncating_zero() { observe(TRUNCATED_CONSTANT as *const u32, 0); }
        pub unsafe fn check_unsigned(n: u8) { observe(n as *const u32, n as usize); }
        pub unsafe fn check_signed(n: i8) { observe(n as *const u32, n as usize); }
        pub unsafe fn check_wide(n: u128) { observe(n as *const u32, n as usize); }
        pub unsafe fn check_raw(p: *const u8) { observe(p as *const u32, p as usize); }
        pub unsafe fn check_slice(p: &[u32]) { observe(p.as_ptr().cast(), p.as_ptr() as usize); }
        pub unsafe fn check_wrapping(n: usize) {
            let p = n as *const u32;
            observe(p.wrapping_add(3), n);
        }
        pub unsafe fn check_wrapping_sub(n: usize) {
            let p = n as *const u32;
            observe(p.wrapping_sub(3), n);
        }
        pub unsafe fn check_overwrite(mut n: usize) {
            let p = n as *const u32;
            n = n.wrapping_add(1);
            observe(p, n);
        }
        pub unsafe fn check_merge(n: usize, flag: bool) {
            let p = if flag { n as *const u32 } else { 1usize as *const u32 };
            observe(p, n);
        }
    "#
        .replace(
            "TRUNCATED_CONSTANT",
            &format!("{}u128", 1u128 << usize::BITS),
        ),
    )
    .unwrap();
    rustc_driver::run_compiler(
        &[
            "rustc".into(),
            source.to_string_lossy().into_owned(),
            "--edition=2021".into(),
            "--crate-name=mirsa_pointer_casts".into(),
            "--crate-type=lib".into(),
            "--emit=metadata".into(),
            "-Zmir-opt-level=0".into(),
        ],
        &mut Checks,
    );
    std::fs::remove_file(source).unwrap();
}
