#![feature(rustc_private)]
extern crate rustc_driver;
extern crate rustc_interface;
extern crate rustc_middle;

use mirsa_analysis::interprocedural::AnalysisSession;
use mirsa_framework::forward::PathForwardAnalysisConfig;
use rustc_driver::{Callbacks, Compilation};
use rustc_middle::{
    mir::TerminatorKind,
    ty::{TyCtxt, TyKind},
};
use std::time::Duration;

struct Checks;
impl Callbacks for Checks {
    fn config(&mut self, config: &mut rustc_interface::interface::Config) {
        config
            .opts
            .unstable_opts
            .mir_enable_passes
            .push(("RemoveStorageMarkers".into(), false));
    }

    fn after_analysis(
        &mut self,
        _: &rustc_interface::interface::Compiler,
        tcx: TyCtxt<'_>,
    ) -> Compilation {
        let mut checked = 0;
        for id in tcx.mir_keys(()) {
            let body = tcx.optimized_mir(id.to_def_id());
            let name = tcx.item_name(id.to_def_id());
            for max_paths in [1, 8] {
                let result = AnalysisSession::new(PathForwardAnalysisConfig {
                    max_paths,
                    widen_after_iterations: Some(8),
                    max_duration: Some(Duration::from_secs(10)),
                })
                .analyze(tcx, body);
                assert!(result.completed, "{name}");
                for (bb, data) in body.basic_blocks.iter_enumerated() {
                    let TerminatorKind::Call { func, .. } = &data.terminator().kind else {
                        continue;
                    };
                    let TyKind::FnDef(callee, _) = func.ty(&body.local_decls, tcx).kind() else {
                        continue;
                    };
                    let expected = match tcx.item_name(*callee).as_str() {
                        "live" => true,
                        "dead" => false,
                        _ => continue,
                    };
                    assert_eq!(
                        result.reachable[bb.index()],
                        expected,
                        "{name} {bb:?}, paths={max_paths}"
                    );
                    checked += 1;
                }
                if name.as_str() == "path_budget" && max_paths == 8 {
                    assert_eq!(result.paths.len(), 2);
                    assert!(result.paths.iter().all(|p| !p.merged));
                }
            }
        }
        assert!(checked >= 40, "only {checked} marker checks");
        Compilation::Stop
    }
}

#[test]
fn symbolic_conditions_prune_only_impossible_paths_before_allocating_slots() {
    let source =
        std::env::temp_dir().join(format!("mirsa-symbolic-branches-{}.rs", std::process::id()));
    std::fs::write(
        &source,
        r#"
unsafe extern "C" { fn live(); fn dead(); fn modify(p: *mut u32); }
pub unsafe fn copy_equality(g: u32) {
    let h = g;
    if g != h { dead(); } else { live(); }
}
pub unsafe fn copy_order(g: i64) {
    let h = g;
    if g < h || g > h { dead(); }
    if g <= h && g >= h && g == h { live(); } else { dead(); }
}
pub unsafe fn guarded_equal(g: u32, h: u32) {
    if g == h { if h != g { dead(); } else { live(); } }
}
pub unsafe fn guarded_unequal(g: u32, h: u32) {
    if g != h { if h == g { dead(); } else { live(); } }
}
pub unsafe fn guarded_order(g: u32, h: u32) {
    if g < h { if h <= g { dead(); } else { live(); } }
}
pub unsafe fn false_order(g: u32, h: u32) {
    if g < h {} else { if h > g { dead(); } else { live(); } }
}
pub unsafe fn antisymmetric(g: u32, h: u32) {
    if g <= h && h <= g { if g != h { dead(); } else { live(); } }
}
pub unsafe fn transitive_equality(g: u32) {
    let h = g; let i = h;
    if i != g { dead(); } else { live(); }
}
pub unsafe fn bool_equality(g: bool) {
    let h = g;
    if g != h { dead(); } else { live(); }
}
pub unsafe fn unknown(g: u32, h: u32) {
    if g == h { live(); } else { live(); }
}
pub unsafe fn reassigned(mut g: u32) {
    let h = g; g = g.wrapping_add(1);
    if g != h { live(); }
}
pub unsafe fn replaced_order(mut g: u32, h: u32) {
    if g < h { g = h; if g == h { live(); } else { dead(); } }
}
pub unsafe fn alias_write(mut g: u32) {
    let h = g; modify(&mut g);
    if g != h { live(); }
}
pub unsafe fn merged(g: u32, flag: bool) {
    let h = if flag { g } else { g.wrapping_add(1) };
    if g != h { live(); }
}
pub unsafe fn nan(g: f64) {
    let h = g;
    if g != h { live(); } else { live(); }
}
pub unsafe fn float_order(g: f64, h: f64) {
    if !(g < h) { if !(g >= h) { live(); } }
}
pub unsafe fn loop_updates() {
    let mut g = 0u32; let h = g;
    while g < 20 { g += 1; if g != h { live(); } }
}
pub unsafe fn path_budget(g: u32, flag: bool) {
    let h = g;
    if g != h { dead(); } if h != g { dead(); }
    if g < h { dead(); } if h > g { dead(); }
    if g == h {} else { dead(); } if g <= h {} else { dead(); }
    if h >= g {} else { dead(); } if h == g {} else { dead(); }
    if flag { live(); } else { live(); }
}
"#,
    )
    .unwrap();
    for opt in [0, 1] {
        rustc_driver::run_compiler(
            &[
                "rustc".into(),
                source.to_string_lossy().into_owned(),
                "--edition=2021".into(),
                "--crate-name=mirsa_symbolic_branches".into(),
                "--crate-type=lib".into(),
                "--emit=metadata".into(),
                format!("-Zmir-opt-level={opt}"),
                "-Zinline-mir=no".into(),
                "-Coverflow-checks=yes".into(),
            ],
            &mut Checks,
        );
    }
    std::fs::remove_file(source).unwrap();
}
