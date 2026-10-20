#![feature(rustc_private)]
extern crate rustc_driver;
extern crate rustc_interface;
extern crate rustc_middle;
extern crate rustc_session;
extern crate rustc_span;

use mirsa::analysis::interprocedural::AnalysisSession;
use mirsa::query::{Bindings, Output, Report};
use mirsa::{Error, Mirsa, Options, SourceBindings};
use rustc_driver::{Callbacks, Compilation};
use rustc_middle::{
    mir::{Location, Place, START_BLOCK},
    ty::TyCtxt,
};

const SOURCE: &str = r#"
    pub fn example(s: &[u8; 4], i: usize) -> u8 {
        // mirsa: start
        if i < 4 { unsafe { *s.get_unchecked(i) } } else { 0 }
    }
"#;
const DSL: &str = "$i >= 0 at 'start";
const FACTS: &str = "facts($i) at 'start";
fn bindings() -> SourceBindings {
    SourceBindings::new().variable("s", "s").variable("i", "i")
}
fn prove(report: &Report, expected: bool) {
    let Output::Check(proven) = &report.output else {
        panic!()
    };
    assert_eq!(proven.result, expected, "{report:#?}");
}

#[test]
fn source_and_mir_adapters_preserve_existing_query_results() {
    let report = Mirsa::from_source(SOURCE).query(DSL, &bindings()).unwrap();
    prove(&report, true);
    let facts_report = Mirsa::from_source(SOURCE)
        .query(FACTS, &bindings())
        .unwrap();
    let Output::Facts(facts) = &facts_report.output else {
        panic!()
    };
    assert!(!facts.abstract_info.is_empty());
    struct Compare {
        expected: [String; 2],
        checked: bool,
    }
    impl Callbacks for Compare {
        fn config(&mut self, config: &mut rustc_interface::interface::Config) {
            config.input = rustc_session::config::Input::Str {
                name: rustc_span::FileName::Custom("mirsa_input.rs".into()),
                input: SOURCE.into(),
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
            let bindings = Bindings::new()
                .value("s", Place::from(args.next().unwrap()))
                .value("i", Place::from(args.next().unwrap()))
                .location(
                    "start",
                    Location {
                        block: START_BLOCK,
                        statement_index: 0,
                    },
                );
            let session = AnalysisSession::new(Options::default().analysis);
            for (dsl, expected) in [DSL, FACTS].into_iter().zip(&self.expected) {
                let original = session
                    .analyze_for_queries(tcx, body)
                    .query(dsl, &bindings)
                    .unwrap();
                let adapted = Mirsa::from_mir(tcx, body).query(dsl, &bindings).unwrap();
                assert_eq!(&format!("{original:?}"), expected);
                assert_eq!(&format!("{adapted:?}"), expected);
            }
            self.checked = true;
            Compilation::Stop
        }
    }
    let mut callback = Compare {
        expected: [format!("{report:?}"), format!("{facts_report:?}")],
        checked: false,
    };
    rustc_driver::run_compiler(
        &[
            "rustc",
            "mirsa_input.rs",
            "--crate-name=mirsa_input",
            "--crate-type=lib",
            "--edition=2021",
            "--emit=metadata",
            "-Zmir-opt-level=0",
            "-Zmir-enable-passes=-RemoveStorageMarkers",
        ]
        .map(String::from),
        &mut callback,
    );
    assert!(callback.checked);
}

#[test]
fn file_input_and_aliases_use_the_same_markers() {
    let text = Mirsa::from_source(SOURCE).query(DSL, &bindings()).unwrap();
    let path = std::env::temp_dir().join(format!("mirsa-input-api-{}.rs", std::process::id()));
    std::fs::write(&path, SOURCE).unwrap();
    let file = Mirsa::from_file(&path)
        .rustc_args(vec!["--edition=2024".into()])
        .query(DSL, &bindings());
    std::fs::remove_file(path).unwrap();
    assert_eq!(format!("{:?}", file.unwrap()), format!("{text:?}"));
}

#[test]
fn marker_selects_its_function_and_ignores_other_bodies_and_fake_comments() {
    let source = r#"
        pub fn unrelated() {
            let text = "// mirsa: point";
            /* // mirsa: point */
            let _closure = || {
                // mirsa: point
                std::hint::black_box(99u32);
            };
            // mirsa: unrelated-invalid-name
            std::hint::black_box(text);
        }
        mod inner {
            pub fn target() {
                let x = 7u32;
                // mirsa: point
                std::hint::black_box(x);
            }
        }
    "#;
    let report = Mirsa::from_source(source)
        .query(
            "$x == 7 at 'point",
            &SourceBindings::new().variable("x", "x"),
        )
        .unwrap();
    prove(&report, true);
}

#[test]
fn duplicate_marker_names_require_unique_names() {
    let source = r#"
        pub fn left() {
            let x = 1u32;
            // mirsa: point
            std::hint::black_box(x);
        }
        mod inner {
            pub fn right() {
                let x = 2u32;
                // mirsa: point
                std::hint::black_box(x);
            }
        }
    "#;
    let bindings = SourceBindings::new().variable("x", "x");
    let Err(Error::Binding(message)) =
        Mirsa::from_source(source).query("$x == 1 at 'point", &bindings)
    else {
        panic!("duplicate markers must not pick a function arbitrarily");
    };
    assert!(message.contains("多个函数") && message.contains("left") && message.contains("right"));
    let source = source
        .replacen("// mirsa: point", "// mirsa: left_point", 1)
        .replacen("// mirsa: point", "// mirsa: right_point", 1);
    for (marker, value) in [("left_point", 1), ("right_point", 2)] {
        let report = Mirsa::from_source(source.clone())
            .query(&format!("$x == {value} at '{marker}"), &bindings)
            .unwrap();
        prove(&report, true);
    }
}

#[test]
fn file_input_finds_markers_in_child_modules() {
    let directory = std::env::temp_dir().join(format!("mirsa-input-module-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let path = directory.join("lib.rs");
    std::fs::write(&path, "mod child; pub fn unrelated() {} ").unwrap();
    std::fs::write(
        directory.join("child.rs"),
        "pub fn target() { let x = 7u32;\n// mirsa: point\nstd::hint::black_box(x); }",
    )
    .unwrap();
    let report = Mirsa::from_file(&path).query(
        "$x == 7 at 'point",
        &SourceBindings::new().variable("x", "x"),
    );
    std::fs::remove_dir_all(directory).unwrap();
    prove(&report.unwrap(), true);
}

#[test]
fn points_before_assignment_branch_return_and_tail_expression() {
    let source = r#"
        pub fn example() -> u32 {
            let mut count = 1;
            // mirsa: assign
            count = 2;
            // mirsa: branch
            if count == 2 {
                // mirsa: ret
                return count;
            }
            count
        }
    "#;
    for (dsl, verdict) in [
        ("$n == 1 at 'assign", true),
        ("$n == 2 at 'assign", false),
        ("$n == 2 at 'branch", true),
        ("$n == 2 at 'ret", true),
    ] {
        let report = Mirsa::from_source(source)
            .query(dsl, &SourceBindings::new().variable("n", "count"))
            .unwrap();
        prove(&report, verdict);
    }
    let source = "pub fn example() -> u32 { let x = 3;\n// mirsa: end\nx\n}";
    prove(
        &Mirsa::from_source(source)
            .query("$x == 3 at 'end", &SourceBindings::new().variable("x", "x"))
            .unwrap(),
        true,
    );
}

#[test]
fn each_marker_resolves_its_own_lexical_scope_and_facts_names() {
    let source = r#"
        pub fn example() {
            let x = 1u32;
            // mirsa: outer
            std::hint::black_box(x);
            {
                let x = 2u32;
                // mirsa: inner
                std::hint::black_box(x);
            }
            // mirsa: after
            std::hint::black_box(x);
        }
    "#;
    for dsl in [
        "$n == 1 at 'outer",
        "$n == 2 at 'inner",
        "$n == 1 at 'after",
    ] {
        let report = Mirsa::from_source(source)
            .query(dsl, &SourceBindings::new().variable("n", "x"))
            .unwrap();
        prove(&report, true);
    }
    let report = Mirsa::from_source(source)
        .query(
            "facts($n) at 'inner",
            &SourceBindings::new().variable("n", "x"),
        )
        .unwrap();
    let Output::Facts(facts) = &report.output else {
        panic!()
    };
    assert!(
        facts
            .abstract_info
            .iter()
            .any(|fact| fact.subject == mirsa::query::Expression::variable("$n"))
    );
}

#[test]
fn separate_markers_select_repeated_calls_with_local_values() {
    let source = r#"
        pub fn example() {
            let s = [1u8, 2, 3, 4];
            let mut i = 0usize;
            // mirsa: first
            std::hint::black_box(unsafe { s.get_unchecked(i) });
            i = 5;
            // mirsa: second
            std::hint::black_box(unsafe { s.get_unchecked(i) });
        }
    "#;
    let first = Mirsa::from_source(source)
        .query("$i < len($s) at 'first", &bindings())
        .unwrap();
    prove(&first, true);
    let second = Mirsa::from_source(source)
        .query("$i < len($s) at 'second", &bindings())
        .unwrap();
    prove(&second, false);
}

#[test]
fn markers_select_the_correct_nested_function() {
    let source = r#"
        pub fn example() {
            fn nested() {
                let x = 2;
                // mirsa: nested_point
                std::hint::black_box(x);
            }
            let _closure = || {
                // mirsa: closure_point
                std::hint::black_box(3);
            };
            let x = 1;
            // mirsa: outer_point
            std::hint::black_box(x);
        }
    "#;
    for (marker, value) in [("outer_point", 1), ("nested_point", 2)] {
        let report = Mirsa::from_source(source)
            .query(
                &format!("$x == {value} at '{marker}"),
                &SourceBindings::new().variable("x", "x"),
            )
            .unwrap();
        prove(&report, true);
    }
}

#[test]
fn markers_are_real_comments_and_binding_errors_are_explicit() {
    let sources = [
        "pub fn example() { let text = \"// mirsa: point\"; std::hint::black_box(text); }",
        "pub fn example() { /* // mirsa: point */ std::hint::black_box(1); }",
        "pub fn example() {\n// mirsa: point\nstd::hint::black_box(1);\n// mirsa: point\nstd::hint::black_box(2); }",
        "pub fn example() {\n// mirsa: point\n}",
        "pub fn example() { std::hint::black_box(1); // mirsa: point\nstd::hint::black_box(2); }",
    ];
    for source in sources {
        assert!(
            matches!(
                Mirsa::from_source(source).query("true at 'point", &SourceBindings::new()),
                Err(Error::Binding(_))
            ),
            "{source}"
        );
    }
    let source = "pub fn example() {\n// mirsa: point\nlet x = 2; std::hint::black_box(x); }";
    assert!(matches!(
        Mirsa::from_source(source).query(
            "$x == 2 at 'point",
            &SourceBindings::new().variable("x", "x")
        ),
        Err(Error::Binding(_))
    ));
    let source = "pub fn example() { { let x = 2; std::hint::black_box(x); }\n// mirsa: point\nstd::hint::black_box(3); }";
    assert!(matches!(
        Mirsa::from_source(source).query(
            "$x == 2 at 'point",
            &SourceBindings::new().variable("x", "x")
        ),
        Err(Error::Binding(_))
    ));
    assert!(matches!(
        Mirsa::from_source(SOURCE).query(DSL, &bindings().variable("i", "i")),
        Err(Error::Binding(_))
    ));
}

#[test]
fn failures_are_results_and_unknown_properties_are_not_errors() {
    assert!(matches!(
        Mirsa::from_source("invalid rust").query("bad DSL", &bindings()),
        Err(Error::Query(_))
    ));
    assert!(matches!(
        Mirsa::from_source("invalid rust").query(DSL, &bindings()),
        Err(Error::Compilation)
    ));
    let report = Mirsa::from_source(SOURCE)
        .query("$i == 0 at 'start", &bindings())
        .unwrap();
    prove(&report, false);
}

#[test]
fn a_query_requires_exactly_one_request() {
    // Invalid DSL must be rejected before trying to compile the input.
    for dsl in [
        "",
        "// comment only",
        "true at 'start;",
        "facts($i) at 'start;",
        "facts() at 'start",
        "facts($i, $s) at 'start",
        "true at 'start; false at 'start;",
        "true at 'start; facts($i) at 'start;",
        "true at 'start false at 'start",
        "true at 'start;;",
    ] {
        assert!(
            matches!(
                Mirsa::from_source("invalid rust").query(dsl, &bindings()),
                Err(Error::Query(_))
            ),
            "{dsl}"
        );
    }
}

#[test]
fn facts_join_alternatives_and_keep_only_common_relations() {
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
            // mirsa: point
            std::hint::black_box((x, related, common, p));
        }
    "#;
    let bindings = SourceBindings::new()
        .variable("x", "x")
        .variable("related", "related")
        .variable("common", "common")
        .variable("base", "base")
        .variable("p", "p");
    let facts = ["x", "related", "common", "p"].map(|name| {
        let report = Mirsa::from_source(source)
            .query(&format!("facts(${name}) at 'point"), &bindings)
            .unwrap();
        let Output::Facts(facts) = report.output else {
            panic!()
        };
        facts
    });
    assert_eq!(
        facts[0]
            .abstract_info
            .iter()
            .find(|f| f.subject == mirsa::query::Expression::variable("$x"))
            .unwrap()
            .value,
        FactValue::Integer {
            low: "1".into(),
            high: "2".into()
        }
    );
    for (fact, name, has_expression) in
        [(&facts[2], "$common", true), (&facts[1], "$related", false)]
    {
        assert_eq!(
            fact.symbolic_info.iter().any(|f| matches!(f, mirsa::query::SymbolicFact::Definition {target, ..} if *target == mirsa::query::Expression::variable(name))),
            has_expression,
            "{fact:?}"
        );
    }
    let FactValue::Pointer { may_be_null, .. } = &facts[3]
        .abstract_info
        .iter()
        .find(|f| f.subject == mirsa::query::Expression::variable("$p"))
        .unwrap()
        .value
    else {
        panic!("{:?}", facts[3]);
    };
    assert!(facts[3].abstract_info.iter().any(|f| matches!(&f.value, FactValue::Allocation {extent_low, extent_high, may_be_live: true, ..} if extent_low == "1" && extent_high == "1")), "{:?}",facts[3]);
    assert!(*may_be_null);
}

#[test]
fn facts_at_an_unreachable_point_do_not_invent_values() {
    let source = r#"
        pub fn example(x: u32) {
            if x < 1 && x > 3 {
                // mirsa: point
                std::hint::black_box(x);
            }
        }
    "#;
    let check = Mirsa::from_source(source)
        .query("true at 'point", &SourceBindings::new())
        .unwrap();
    prove(&check, false);
    let report = Mirsa::from_source(source)
        .query(
            "facts($x) at 'point",
            &SourceBindings::new().variable("x", "x"),
        )
        .unwrap();
    let Output::Facts(facts) = report.output else {
        panic!()
    };
    assert_eq!(facts.abstract_info.len(), 1);
    assert_eq!(
        facts.abstract_info[0].subject,
        mirsa::query::Expression::variable("$x")
    );
    assert_eq!(
        facts.abstract_info[0].value,
        mirsa::query::FactValue::Unreachable
    );
    assert!(facts.symbolic_info.is_empty());
}

#[test]
fn structured_float_facts_preserve_nan_and_owned_values() {
    use mirsa::query::{Expression, FactValue};
    let source = r#"
        pub fn example(flag: bool) {
            let x = if flag { f64::NAN } else { 1.0 };
            let y = x;
            // mirsa: point
            std::hint::black_box((x, y));
        }
    "#;
    let report = Mirsa::from_source(source)
        .query(
            "facts($x) at 'point",
            &SourceBindings::new().variable("x", "x"),
        )
        .unwrap();
    let Output::Facts(facts) = report.output else {
        panic!()
    };
    assert!(
        facts
            .abstract_info
            .iter()
            .any(|f| f.subject == Expression::variable("$x")
                && matches!(f.value, FactValue::Float { may_nan: true, .. })),
        "{facts:#?}"
    );
    // Compiler context has ended; structured expressions and their formatting remain usable.
    let text = facts.to_string();
    assert!(text.contains("抽象信息") && text.contains("符号信息"));
}
