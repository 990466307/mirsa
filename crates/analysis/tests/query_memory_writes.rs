#![feature(rustc_private)]
#![cfg(feature = "z3")]

extern crate rustc_driver;
extern crate rustc_hir;
extern crate rustc_interface;
extern crate rustc_middle;

use mirsa_analysis::interprocedural::AnalysisSession;
use mirsa_analysis::query::{AnalysisView, Bindings, Verdict};
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
            let name = tcx.item_name(id.to_def_id());
            let (valid, stale) = match name.as_str() {
                "check_tuple" | "check_nested" | "check_self_copy" => {
                    ("$a == 3 && $b == 4", "$a == 1 && $b == 2")
                }
                "check_branch" => ("$a + $b == 1", "$a == 0 && $b == 0"),
                "check_closure"
                | "check_mut_closure"
                | "check_holder"
                | "check_array"
                | "check_drop"
                | "check_float_unknown" => ("$b == 17", "$a == 1"),
                "check_slice_input" => ("$a == 1 && $b == 17", "$a == 2"),
                "check_slice_contradiction" => ("true", "false"),
                "check_rebind" | "check_rebind_call" => ("$a == 2 && $b == 2", "$a == 3"),
                _ => continue,
            };
            let body = tcx.optimized_mir(id.to_def_id());
            let config = PathForwardAnalysisConfig {
                max_paths: 8,
                widen_after_iterations: Some(8),
                max_duration: Some(Duration::from_secs(10)),
            };
            let full_session = AnalysisSession::new(config);
            let result = full_session.analyze(tcx, body);
            assert!(result.completed, "{name}");
            let full = AnalysisView::new(tcx, body, &result);
            let sliced_session = AnalysisSession::new(config);
            let sliced = sliced_session.analyze_for_queries(tcx, body);
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
                    .value("a", args[0].node.clone())
                    .value("b", args[1].node.clone());
                for view in [&full, &sliced] {
                    for (property, verdict) in [(valid, Verdict::T), (stale, Verdict::F)] {
                        let proof = view.check(property, at, &bindings).unwrap();
                        if name.as_str() == "check_slice_contradiction" {
                            assert!(proof.unreachable, "{name}: {proof:?}");
                            continue;
                        }
                        assert!(!proof.unreachable, "{name}: {proof:?}");
                        assert_eq!(proof.verdict, verdict, "{name} {property}: {proof:?}");
                    }
                }
                checked += 1;
            }
        }
        assert_eq!(checked, 14);
        Compilation::Stop
    }
}

fn check(opt: u8) {
    let source = std::env::temp_dir().join(format!(
        "mirsa-memory-writes-{}-{opt}.rs",
        std::process::id()
    ));
    std::fs::write(
        &source,
        r#"
struct Holder<'a> { value: (&'a mut i32,) }
struct Flag { value: bool }
struct Guard<'a>(&'a mut i32);
impl Drop for Guard<'_> { fn drop(&mut self) { *self.0 = 100; } }
unsafe extern "C" {
    static UNKNOWN_FLOAT: f32;
    fn observe(a: i32, b: i32);
    fn opaque_holder(v: Holder<'_>);
    fn opaque_array(v: [*mut i32; 1]);
}
unsafe extern "Rust" { fn unknown_slice() -> &'static mut [u8]; }
fn equal_lengths(src: &[u8], dst: &mut [u8]) { assert_eq!(src.len(), dst.len()); }
fn contradictory_lengths(src: &[u8], dst: &mut [u8]) {
    assert_eq!(src.len(), dst.len());
    assert_ne!(src.len(), dst.len());
}
pub unsafe fn check_slice_input() {
    let dst = unknown_slice();
    equal_lengths(&[0; 4], dst);
    observe(1, 17);
}
pub unsafe fn check_slice_contradiction() {
    let dst = unknown_slice();
    contradictory_lengths(&[0; 4], dst);
    observe(1, 17);
}
pub unsafe fn check_float_unknown() {
    observe((UNKNOWN_FLOAT == 15.6) as i32, 17);
}
fn rebind<'a>(dst: &mut &'a i32, src: &'a i32) { *dst = src; }
pub unsafe fn check_rebind() {
    let x = 1;
    let z = 2;
    let mut p = &x;
    let old = *p;
    let q = &x;
    p = &z;
    observe(*q + old, *p);
}
pub unsafe fn check_rebind_call() {
    let x = 1;
    let z = 2;
    let mut p = &x;
    let old = *p;
    let q = &x;
    rebind(&mut p, &z);
    observe(*q + old, *p);
}
fn copy_pair(dst: &mut (i32, i32), src: &(i32, i32)) { *dst = *src; }
fn call_once(f: impl FnOnce()) { f(); }
pub unsafe fn check_tuple() {
    let mut dst = (1, 2);
    let src = (3, 4);
    copy_pair(&mut dst, &src);
    observe(dst.0, dst.1);
}
pub unsafe fn check_nested() {
    let mut dst = ((1,), 2);
    let p = &mut dst;
    *p = ((3,), 4);
    observe(dst.0.0, dst.1);
}
pub unsafe fn check_self_copy() {
    let mut dst = (3, 4);
    let p = &mut dst;
    *p = *p;
    observe(dst.0, dst.1);
}
pub unsafe fn check_branch(cond: bool) {
    let mut left = Flag { value: false };
    let mut right = Flag { value: false };
    let p = if cond { &mut left } else { &mut right };
    *p = Flag { value: true };
    observe(left.value as i32, right.value as i32);
}
pub unsafe fn check_closure() {
    let mut x = 1;
    let mut untouched = 17;
    let keep = &mut untouched;
    call_once(|| x = 2);
    observe(x, *keep);
}
pub unsafe fn check_mut_closure() {
    let mut x = 1;
    let mut untouched = 17;
    let keep = &mut untouched;
    let mut f = |n| x = n;
    f(2);
    observe(x, *keep);
}
pub unsafe fn check_holder() {
    let mut x = 1;
    let mut untouched = 17;
    let keep = &mut untouched;
    opaque_holder(Holder { value: (&mut x,) });
    observe(x, *keep);
}
pub unsafe fn check_array() {
    let mut x = 1;
    let mut untouched = 17;
    let keep = &mut untouched;
    opaque_array([&mut x]);
    observe(x, *keep);
}
pub unsafe fn check_drop() {
    let mut x = 1;
    let mut untouched = 17;
    let keep = &mut untouched;
    { let _guard = Guard(&mut x); }
    observe(x, *keep);
}
"#,
    )
    .unwrap();
    rustc_driver::run_compiler(
        &[
            "rustc".into(),
            source.to_string_lossy().into_owned(),
            "--edition=2021".into(),
            "--crate-name=mirsa_memory_writes".into(),
            "--crate-type=lib".into(),
            "--emit=metadata".into(),
            "--cap-lints=allow".into(),
            "-Zinline-mir=no".into(),
            format!("-Zmir-opt-level={opt}"),
        ],
        &mut Checks,
    );
    std::fs::remove_file(source).unwrap();
}

#[test]
fn memory_writes_opt0() {
    check(0);
}

#[test]
fn memory_writes_opt1() {
    check(1);
}
