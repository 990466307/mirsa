#![feature(rustc_private)]
extern crate rustc_driver;
extern crate rustc_hir;
extern crate rustc_interface;
extern crate rustc_middle;

use mirsa_analysis::interprocedural::AnalysisSession;
use mirsa_analysis::query::{AnalysisView, Bindings, QueryConfig, Verdict};
use mirsa_framework::forward::PathForwardAnalysisConfig;
use rustc_driver::{Callbacks, Compilation};
use rustc_middle::mir::{Location, Operand, ProjectionElem, TerminatorKind};
use rustc_middle::ty::{TyKind, TypingEnv};
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
            let result = AnalysisSession::new(PathForwardAnalysisConfig {
                max_paths: if name == "check_alias" { 1 } else { 8 },
                widen_after_iterations: Some(8),
                max_duration: Some(Duration::from_secs(10)),
            })
            .analyze(tcx, body);
            assert!(result.completed);
            let view = AnalysisView::new(tcx, body, &result).with_config(QueryConfig {
                use_solver: false,
                ..Default::default()
            });
            for (bb, data) in body.basic_blocks.iter_enumerated() {
                let TerminatorKind::Call { func, args, .. } = &data.terminator().kind else {
                    continue;
                };
                let TyKind::FnDef(callee, _) = func.ty(&body.local_decls, tcx).kind() else {
                    continue;
                };
                if tcx.item_name(*callee).as_str() != "observe" {
                    continue;
                }
                let point = Location {
                    block: bb,
                    statement_index: data.statements.len(),
                };
                let values = Bindings::new()
                    .value("r", args[0].node.clone())
                    .value("tag", args[1].node.clone())
                    .value("expected", args[2].node.clone());
                if name == "check_alias" {
                    let item = args[0].node.place().unwrap().project_deeper(
                        &[
                            ProjectionElem::Downcast(None, 1u32.into()),
                            ProjectionElem::Field(0u32.into(), tcx.types.i16),
                        ],
                        tcx,
                    );
                    let values = values.value("item", item);
                    let proof = view.check("$item == $expected", point, &values).unwrap();
                    assert!(!proof.unreachable);
                    assert_ne!(proof.verdict, Verdict::T, "{proof:?}");
                } else if name == "check_unknown" {
                    let proof = view
                        .check("discriminant($r) == 1isize", point, &values)
                        .unwrap();
                    assert!(!proof.unreachable);
                    assert_eq!(proof.verdict, Verdict::F, "{proof:?}");
                    assert!(!proof.definite_violation);
                    assert!(proof.reason.is_some());
                } else {
                    let proof = view
                        .check("discriminant($r) == $tag", point, &values)
                        .unwrap();
                    assert!(!proof.unreachable, "{name}: {proof:?}");
                    assert_eq!(proof.verdict, Verdict::T, "{name} {point:?}: {proof:?}");
                    let Operand::Constant(tag) = &args[1].node else {
                        panic!("constant expected")
                    };
                    if tag
                        .const_
                        .try_eval_scalar_int(tcx, TypingEnv::fully_monomorphized())
                        .unwrap()
                        .to_bits_unchecked()
                        == 1
                    {
                        let place = args[0].node.place().unwrap();
                        let item = place.project_deeper(
                            &[
                                ProjectionElem::Downcast(None, 1u32.into()),
                                ProjectionElem::Field(0u32.into(), tcx.types.i16),
                            ],
                            tcx,
                        );
                        let values = values.value("item", item);
                        let proof = view.check("$item == $expected", point, &values).unwrap();
                        assert_eq!(proof.verdict, Verdict::T, "{name} {point:?}: {proof:?}");
                    }
                }
                checked += 1;
            }
        }
        assert_eq!(checked, 21);
        Compilation::Stop
    }
}

#[test]
fn ranges_preserve_bounds_and_mutate_iterator_state() {
    let source =
        std::env::temp_dir().join(format!("mirsa-range-iterator-{}.rs", std::process::id()));
    std::fs::write(&source,r#"
        unsafe extern "C" { fn observe(r: Option<i16>, tag: isize, expected: i16); }
        pub unsafe fn check_exclusive() { let mut r=1i16..3; observe(r.next(),1,1); observe(r.next(),1,2); observe(r.next(),0,0); }
        pub unsafe fn check_inclusive() { let mut r=1i16..=2; observe(r.next(),1,1); observe(r.next(),1,2); observe(r.next(),0,0); }
        pub unsafe fn check_empty() { let mut r=3i16..=2; observe(r.next(),0,0); observe(r.next(),0,0); }
        pub unsafe fn check_maximum() { let mut r=i16::MAX..=i16::MAX; observe(r.next(),1,i16::MAX); observe(r.next(),0,0); }
        pub unsafe fn check_negative() { let mut r=-2i16..=0; observe(r.next(),1,-2); observe(r.next(),1,-1); observe(r.next(),1,0); observe(r.next(),0,0); }
        pub unsafe fn check_identity() { let mut r=(1i16..=2).into_iter(); observe(r.next(),1,1); observe(r.next(),1,2); observe(r.next(),0,0); }
        pub unsafe fn check_borrow() { let mut original=4i16..=5; let mut r=(&mut original).into_iter(); observe(Iterator::next(&mut r),1,4); observe(original.next(),1,5); }
        pub unsafe fn check_unknown(a:i16,b:i16) { let mut r=a..=b; observe(r.next(),0,0); }
        // A merged receiver may modify either range; retaining left's old start is unsound.
        pub unsafe fn check_alias(flag:bool) {
            let mut left=0i16..2;
            let mut right=10i16..12;
            let selected=if flag { &mut left } else { &mut right };
            let _=selected.next();
            observe(left.next(),1,0);
        }
        pub fn check_chars() { let _ = "abc".chars().into_iter().next(); }
    "#).unwrap();
    rustc_driver::run_compiler(
        &[
            "rustc".into(),
            source.to_string_lossy().into_owned(),
            "--edition=2021".into(),
            "--crate-name=mirsa_range_iterator".into(),
            "--crate-type=lib".into(),
            "--emit=metadata".into(),
            "--cap-lints=allow".into(),
            "-Zmir-opt-level=0".into(),
        ],
        &mut Checks,
    );
    std::fs::remove_file(source).unwrap();
}
