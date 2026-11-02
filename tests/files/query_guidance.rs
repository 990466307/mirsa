#![feature(rustc_private)]
#![cfg(feature = "z3")]
extern crate rustc_driver;
extern crate rustc_interface;
extern crate rustc_middle;

use mirsa_analysis::{interprocedural::AnalysisSession, query::*};
use mirsa_framework::forward::PathForwardAnalysisConfig;
use rustc_driver::{Callbacks, Compilation};
use rustc_middle::mir::*;
use std::time::Duration;

struct Checks;
impl Callbacks for Checks {
    fn after_analysis<'tcx>(
        &mut self,
        _: &rustc_interface::interface::Compiler,
        tcx: rustc_middle::ty::TyCtxt<'tcx>,
    ) -> Compilation {
        let mut recovered = 0;
        for id in tcx.mir_keys(()) {
            let name = tcx.item_name(id.to_def_id());
            if !["safe", "bad", "overwrite", "mutated"].contains(&name.as_str()) {
                continue;
            }
            let body = tcx.optimized_mir(id.to_def_id());
            let cfg = mirsa_core::cfg::build_cfg(body);
            for mode in ["baseline", "correct", "wrong"] {
                let mut candidates = Vec::new();
                if mode != "baseline" {
                    for (bb, data) in body.basic_blocks.iter_enumerated() {
                        if data.is_cleanup || cfg.pred[bb.index()].len() < 2 {
                            continue;
                        }
                        for (local, decl) in body.local_decls.iter_enumerated() {
                            if !decl.ty.is_raw_ptr() {
                                continue;
                            }
                            let bindings = Bindings::new().value("p", Place::from(local));
                            let expr = if mode == "correct" {
                                "address($p) % 4 == 0"
                            } else {
                                "address($p) % 4 == 1"
                            };
                            candidates
                                .push(Candidate::new(tcx, body, bb, expr, &bindings).unwrap());
                            assert!(
                                Candidate::new(tcx, body, bb, "is_live($p)", &bindings).is_err()
                            );
                        }
                    }
                }
                let session = AnalysisSession::new(PathForwardAnalysisConfig {
                    max_paths: 1,
                    widen_after_iterations: Some(8),
                    max_duration: Some(Duration::from_secs(30)),
                })
                .with_guidance(candidates, QueryConfig::default());
                let result = session.analyze(tcx, body);
                assert!(result.completed, "{name} {mode}");
                for (bb, data) in body.basic_blocks.iter_enumerated() {
                    let TerminatorKind::Call { func, args, .. } = &data.terminator().kind else {
                        continue;
                    };
                    let rustc_middle::ty::FnDef(callee, _) = func.ty(&body.local_decls, tcx).kind()
                    else {
                        continue;
                    };
                    if tcx.item_name(*callee).as_str() != "observe" {
                        continue;
                    }
                    let bindings = Bindings::new().value("p", args[0].node.clone());
                    let proof = AnalysisView::new(tcx, body, &result)
                        .check(
                            "address($p) % 4 == 0",
                            Location {
                                block: bb,
                                statement_index: data.statements.len(),
                            },
                            &bindings,
                        )
                        .unwrap();
                    let expected = if name.as_str() == "safe" && mode == "correct" {
                        Verdict::T
                    } else {
                        Verdict::F
                    };
                    assert_eq!(
                        proof.verdict,
                        expected,
                        "{name} {mode}: {proof:?}; {:?}",
                        session.guidance_checks()
                    );
                    if expected == Verdict::T {
                        recovered += 1;
                        assert!(session.guidance_checks().0.iter().any(|c| c.accepted));
                    }
                    if mode == "wrong" {
                        assert!(session.guidance_checks().0.iter().any(|c| !c.accepted));
                    }
                }
            }
        }
        assert_eq!(recovered, 1);
        Compilation::Stop
    }
}
#[test]
fn validated_relations_survive_joins_but_not_writes() {
    let source = std::env::temp_dir().join(format!("mirsa-guidance-{}.rs", std::process::id()));
    std::fs::write(&source,r#"
        extern "C" { fn observe(p:*const u8); fn mutate(p:*mut *const u8); }
        pub unsafe fn safe(flag:bool) { let p=if flag {4usize as *const u8} else {8usize as *const u8}; observe(p); }
        pub unsafe fn bad(flag:bool) { let p=if flag {4usize as *const u8} else {5usize as *const u8}; observe(p); }
        pub unsafe fn overwrite(flag:bool) { let mut p=if flag {4usize as *const u8} else {8usize as *const u8}; p=1usize as *const u8; observe(p); }
        pub unsafe fn mutated(flag:bool) { let mut p=if flag {4usize as *const u8} else {8usize as *const u8}; mutate(&mut p); observe(p); }
    "#).unwrap();
    rustc_driver::run_compiler(
        &[
            "rustc".into(),
            source.to_string_lossy().into_owned(),
            "--edition=2021".into(),
            "--crate-name=guidance".into(),
            "--crate-type=lib".into(),
            "--emit=metadata".into(),
            "-Zmir-opt-level=0".into(),
        ],
        &mut Checks,
    );
    std::fs::remove_file(source).unwrap();
}
