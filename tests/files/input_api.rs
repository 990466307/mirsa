#![feature(rustc_private)]
extern crate rustc_driver;
extern crate rustc_interface;
extern crate rustc_middle;
extern crate rustc_session;
extern crate rustc_span;
use mirsa::query::Output;
use mirsa::{Error, Mirsa, Options};

#[test]
fn annotated_file_and_memory_input_produce_the_same_reports() {
    let source = "pub fn example(x:u8) { #[mirsa::check(x >= 0)] #[mirsa::facts(x)] std::hint::black_box(x); }";
    let memory = Mirsa::from_source(source).analyze_annotations().unwrap();
    let path = std::env::temp_dir().join(format!("mirsa-input-api-{}.rs", std::process::id()));
    std::fs::write(&path, source).unwrap();
    let disk = Mirsa::from_file(&path)
        .rustc_args(vec!["--edition=2024".into()])
        .analyze_annotations();
    std::fs::remove_file(path).unwrap();
    let disk = disk.unwrap();
    assert_eq!(memory.len(), disk.len());
    for (a, b) in memory.iter().zip(&disk) {
        assert_eq!(format!("{:?}", a.report), format!("{:?}", b.report));
    }
}

#[test]
fn comment_markers_and_attribute_text_inside_strings_do_not_create_queries() {
    let source = r##"pub fn example() {
        // mirsa: point
        let text = "#[mirsa::check(missing)]";
        /* #[mirsa::facts(missing)] */
        std::hint::black_box(text);
    }"##;
    assert!(
        Mirsa::from_source(source)
            .analyze_annotations()
            .unwrap()
            .is_empty()
    );
}

#[test]
fn compilation_and_attribute_type_errors_remain_diagnostics() {
    assert!(matches!(
        Mirsa::from_source("invalid rust").analyze_annotations(),
        Err(Error::Compilation)
    ));
    let source = "pub fn example(i: usize) { #[mirsa::check(i)] std::hint::black_box(i); }";
    let Err(Error::Query(errors)) = Mirsa::from_source(source).analyze_annotations() else {
        panic!()
    };
    assert!(errors[0].message.contains("usize") && errors[0].message.contains("bool"));
    assert!(errors[0].hint.as_ref().unwrap().contains("facts 查询"));
    assert_eq!(errors[0].context.as_ref().unwrap().source_line, "i");
}

#[test]
fn missing_attribute_variables_list_only_visible_names() {
    let source = r#"
        pub fn example(arg: u32) {
            let present = 1u32;
            { let hidden = 2u32; std::hint::black_box(hidden); }
            #[mirsa::check(nope > 0)]
            let future = present + arg;
            std::hint::black_box(future);
        }
    "#;
    let Err(Error::Binding(message)) = Mirsa::from_source(source).analyze_annotations() else {
        panic!("expected an unresolved variable");
    };
    assert!(
        message.contains("nope") && message.contains("可见变量"),
        "{message}"
    );
    assert!(
        message.contains("arg") && message.contains("present"),
        "{message}"
    );
    assert!(
        !message.contains("hidden") && !message.contains("future"),
        "{message}"
    );
}

#[test]
fn low_level_mir_adapter_still_matches_the_analysis_engine() {
    use rustc_driver::{Callbacks, Compilation};
    use rustc_middle::{mir::*, ty::TyCtxt};
    struct Compare(bool);
    impl Callbacks for Compare {
        fn config(&mut self, config: &mut rustc_interface::interface::Config) {
            config.input=rustc_session::config::Input::Str {
                name:rustc_span::FileName::Custom("mirsa_input.rs".into()),
                input:"pub fn example(s:&[u8;4],i:usize)->u8 { if i<4 { unsafe{*s.get_unchecked(i)} } else {0} }".into(),
            };
        }
        fn after_analysis<'tcx>(
            &mut self,
            _: &rustc_interface::interface::Compiler,
            tcx: TyCtxt<'tcx>,
        ) -> Compilation {
            let id = tcx
                .mir_keys(())
                .iter()
                .find(|id| tcx.item_name(id.to_def_id()).as_str() == "example")
                .unwrap();
            let body = tcx.optimized_mir(id.to_def_id());
            let mut args = body.args_iter();
            let bindings = mirsa::query::Bindings::new()
                .value("s", Place::from(args.next().unwrap()))
                .value("i", Place::from(args.next().unwrap()))
                .location(
                    "start",
                    Location {
                        block: START_BLOCK,
                        statement_index: 0,
                    },
                );
            let session =
                mirsa::analysis::interprocedural::AnalysisSession::new(Options::default().analysis);
            for dsl in ["$i >= 0 at 'start", "facts($s) at 'start"] {
                let original = session
                    .analyze_for_queries(tcx, body)
                    .query(dsl, &bindings)
                    .unwrap();
                let adapted = Mirsa::from_mir(tcx, body).query(dsl, &bindings).unwrap();
                assert_eq!(format!("{original:?}"), format!("{adapted:?}"));
            }
            self.0 = true;
            Compilation::Stop
        }
    }
    let mut callback = Compare(false);
    rustc_driver::run_compiler(
        &[
            "rustc",
            "mirsa_input.rs",
            "--crate-name=mirsa_input",
            "--crate-type=lib",
            "--emit=metadata",
            "-Zmir-opt-level=0",
        ]
        .map(String::from),
        &mut callback,
    );
    assert!(callback.0);
}

#[test]
fn facts_preserve_path_values_and_relations() {
    use mirsa::query::FactValue;
    let source = r#"
        pub fn example(flag: bool, y: u8) {
            let base = y as u32;
            let common = base + 1;
            let x;
            let related;
            let p: *const u8;
            if flag { x = 1u32; related = base + 1; p = std::ptr::null(); }
            else { x = 2u32; related = base + 2; p = &y; }
            #[mirsa::facts(x)]
            #[mirsa::facts(related)]
            #[mirsa::facts(common)]
            #[mirsa::facts(p)]
            std::hint::black_box((x, related, common, p));
        }
    "#;
    let facts = Mirsa::from_source(source)
        .analyze_annotations()
        .unwrap()
        .into_iter()
        .map(|r| match r.report.output {
            Output::Facts(facts) => facts,
            _ => panic!(),
        })
        .collect::<Vec<_>>();
    for output in &facts {
        assert!(!output.unreachable);
        assert!(output.paths.len() >= 2, "{output:?}");
    }
    let mut values = facts[0]
        .paths
        .iter()
        .map(|path| {
            let fact = path
                .abstract_info
                .iter()
                .find(|f| f.subject == mirsa::query::Expression::variable("$x"))
                .unwrap();
            let FactValue::Integer { low, high } = &fact.value else {
                panic!("{fact:?}");
            };
            assert_eq!(low, high);
            low.clone()
        })
        .collect::<Vec<_>>();
    values.sort();
    values.dedup();
    assert_eq!(values, ["1", "2"]);
    for path in &facts[1].paths {
        assert!(path.symbolic_info.iter().any(|f| matches!(f, mirsa::query::SymbolicFact::Definition { target, .. } if *target == mirsa::query::Expression::variable("$related"))), "{path:?}");
    }
    let nullness = facts[3]
        .paths
        .iter()
        .map(|path| {
            let fact = path
                .abstract_info
                .iter()
                .find(|f| f.subject == mirsa::query::Expression::variable("$p"))
                .unwrap();
            let FactValue::Pointer { may_be_null, .. } = fact.value else {
                panic!("{fact:?}");
            };
            may_be_null
        })
        .collect::<Vec<_>>();
    assert!(nullness.contains(&true) && nullness.contains(&false));
}

#[test]
fn facts_at_an_unreachable_point_do_not_invent_values() {
    let source = r#"
        pub fn example(x: u32) {
            if x < 1 && x > 3 {
                #[mirsa::facts(x)]
                std::hint::black_box(x);
            }
        }
    "#;
    let report = Mirsa::from_source(source)
        .analyze_annotations()
        .unwrap()
        .remove(0)
        .report;
    let Output::Facts(facts) = report.output else {
        panic!()
    };
    assert!(facts.unreachable);
    assert!(facts.paths.is_empty());
}

#[test]
fn structured_float_facts_preserve_nan_and_owned_values() {
    use mirsa::query::{Expression, FactValue};
    let source = r#"
        pub fn example(flag: bool) {
            let x = if flag { f64::NAN } else { 1.0 };
            let y = x;
            #[mirsa::facts(x)]
            std::hint::black_box((x, y));
        }
    "#;
    let report = Mirsa::from_source(source)
        .analyze_annotations()
        .unwrap()
        .remove(0)
        .report;
    let Output::Facts(facts) = report.output else {
        panic!()
    };
    assert!(
        facts
            .paths
            .iter()
            .flat_map(|path| &path.abstract_info)
            .any(|f| f.subject == Expression::variable("$x")
                && matches!(f.value, FactValue::Float { may_nan: true, .. })),
        "{facts:#?}"
    );
    // Compiler context has ended; structured expressions and their formatting remain usable.
    let text = facts.to_string();
    assert!(text.contains("抽象信息") && text.contains("符号信息"));
}

#[test]
fn property_evidence_explains_true_false_and_unknown() {
    use mirsa::query::{Evaluation, EvaluationRule, FactValue, Verdict};
    fn comparison(e: &Evaluation) -> Option<&Evaluation> {
        if matches!(e.rule, EvaluationRule::Comparison) {
            return Some(e);
        }
        e.premises.iter().find_map(comparison)
    }
    let source = r#"
        pub fn example(x: u8) {
            if x < 8 {
                // mirsa: point
                std::hint::black_box(x);
            }
        }
    "#;
    for (query, expected, definite) in [
        ("$x < 8", Some(true), false),
        ("$x >= 8", Some(false), true),
        ("$x == 3", None, false),
    ] {
        let annotated = source.replace(
            "// mirsa: point",
            &format!("#[mirsa::check({})]", query.replace('$', "")),
        );
        let report = Mirsa::from_source(annotated)
            .analyze_annotations()
            .unwrap()
            .remove(0)
            .report;
        let Output::Check(proof) = &report.output else {
            panic!();
        };
        assert_eq!(
            proof.verdict,
            if expected == Some(true) {
                Verdict::T
            } else {
                Verdict::F
            }
        );
        assert_eq!(proof.definite_violation, definite);
        assert!(!proof.paths.is_empty());
        for path in &proof.paths {
            let root = path.evidence.abstract_evaluation.as_ref().unwrap();
            assert_eq!(root.value, FactValue::Bool { value: expected });
            let step = comparison(root).unwrap();
            assert_eq!(step.premises.len(), 2);
            assert_eq!(step.premises[0].expression.to_string(), "$x");
            assert_eq!(
                step.premises[0].value,
                FactValue::Integer {
                    low: "0".into(),
                    high: "7".into()
                }
            );
            assert_eq!(step.value, FactValue::Bool { value: expected });
        }
        // The evidence remains usable after the compiler session ends.
        assert!(format!("{proof:?}").contains("abstract_evaluation"));
    }
}
