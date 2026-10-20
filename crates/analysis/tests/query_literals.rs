#![feature(rustc_private)]

extern crate rustc_driver;
extern crate rustc_interface;
extern crate rustc_middle;

use mirsa_analysis::interprocedural::AnalysisSession;
use mirsa_analysis::query::{Bindings, Output, Verdict};
use mirsa_framework::forward::PathForwardAnalysisConfig;
use rustc_driver::{Callbacks, Compilation};
use std::time::Duration;

struct QueryChecks;

impl Callbacks for QueryChecks {
    fn after_analysis<'tcx>(
        &mut self,
        _: &rustc_interface::interface::Compiler,
        tcx: rustc_middle::ty::TyCtxt<'tcx>,
    ) -> Compilation {
        let body = tcx.optimized_mir(tcx.entry_fn(()).unwrap().0);
        let session = AnalysisSession::new(PathForwardAnalysisConfig {
            max_paths: 10,
            widen_after_iterations: Some(10),
            max_duration: Some(Duration::from_secs(10)),
        });
        let view = session.analyze_for_queries(tcx, body);
        let bindings = Bindings::new().location(
            "p",
            rustc_middle::mir::Location {
                block: rustc_middle::mir::START_BLOCK,
                statement_index: 0,
            },
        );
        // Compare against rustc's native literal semantics, not a duplicate
        // implementation of the query parser's conversion algorithm.
        let cases = [
            (
                "1.0000000596046448f32 == 1.00000011920928955078125f32",
                1.0000000596046448f32 == 1.00000011920928955078125f32,
            ),
            (
                "-1.0000000596046448f32 == -1.00000011920928955078125f32",
                -1.0000000596046448f32 == -1.00000011920928955078125f32,
            ),
            (
                "1.0000000596046448f32 == 1.0f32",
                1.0000000596046448f32 == 1.0f32,
            ),
            (
                "1.0000000596046448f64 == 1.00000011920928955078125f64",
                1.0000000596046448f64 == 1.00000011920928955078125f64,
            ),
            ("1u8 + 2u8 == 3u8", true),
        ];
        for (expression, expected) in cases {
            let report = view
                .query(&format!("{expression} at 'p"), &bindings)
                .unwrap();
            let Output::Check(check) = &report.output else {
                panic!("expected a check result");
            };
            assert_eq!(check.result, expected, "{expression}");
            let direct = view
                .check(
                    expression,
                    rustc_middle::mir::Location {
                        block: rustc_middle::mir::START_BLOCK,
                        statement_index: 0,
                    },
                    &Bindings::new(),
                )
                .unwrap();
            assert_eq!(direct.verdict == Verdict::T, check.result);
            assert_eq!(direct.definite_violation, !expected);
            assert!(!direct.unreachable);
        }
        // The interval domain merges signed zeros; SMT retains their IEEE
        // semantics. Without the backend this query must stay conservative.
        let report = view
            .query("1.0f32 / -0.0f32 < 0.0f32 at 'p", &bindings)
            .unwrap();
        let Output::Check(check) = &report.output else {
            panic!("expected a check result");
        };
        assert_eq!(check.result, cfg!(feature = "z3"));
        let report = view.query("1u8 / 0u8 == 255u8 at 'p", &bindings).unwrap();
        let Output::Check(check) = &report.output else {
            panic!("expected a check result");
        };
        assert!(!check.result);
        let proof = view
            .check(
                "1u8 / 0u8 == 255u8",
                rustc_middle::mir::Location {
                    block: rustc_middle::mir::START_BLOCK,
                    statement_index: 0,
                },
                &Bindings::new(),
            )
            .unwrap();
        assert_eq!(proof.verdict, Verdict::F);
        assert!(!proof.definite_violation);
        assert!(proof.reason.is_some());
        Compilation::Stop
    }
}

#[test]
fn query_literals_match_rust_semantics() {
    let source =
        std::env::temp_dir().join(format!("mirsa-query-literals-{}.rs", std::process::id()));
    std::fs::write(&source, "fn main() {}\n").unwrap();
    rustc_driver::run_compiler(
        &[
            "rustc".into(),
            source.to_string_lossy().into_owned(),
            "--crate-name=mirsa_query_literals".into(),
            "--emit=metadata".into(),
            "-Zmir-opt-level=0".into(),
        ],
        &mut QueryChecks,
    );
    std::fs::remove_file(source).unwrap();
}
