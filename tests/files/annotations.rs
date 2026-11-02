#![feature(rustc_private)]
use mirsa::{
    Mirsa,
    query::{Output, Verdict},
};

#[test]
fn annotations_resolve_builtins_locals_shadowing_and_multiple_requests() {
    let source = r#"
        pub fn example(len: usize) {
            let table = [1u8, 2, 3];
            let index = 5usize;
            #[mirsa::facts(index)]
            #[mirsa::check(index < len(table))]
            #[mirsa::check(len >= 0)]
            let value = unsafe { *table.get_unchecked(index) };
            {
                let index = 1usize;
                #[mirsa::check(index < len(table))]
                std::hint::black_box(index);
            }
            std::hint::black_box(value);
        }
    "#;
    let reports = Mirsa::from_source(source).analyze_annotations().unwrap();
    assert_eq!(reports.len(), 4, "{reports:?}");
    assert!(matches!(&reports[0].report.output, Output::Facts(f) if !f.paths.is_empty()));
    let expected = [(1, false, true), (2, true, false), (3, true, false)];
    for (i, proven, violation) in expected {
        let Output::Check(check) = &reports[i].report.output else {
            panic!()
        };
        assert_eq!(check.result, proven, "{:?}", reports[i]);
        assert_eq!(check.definite_violation, violation);
        assert!(check.paths[0].evidence.expressions.is_some());
        assert!(reports[i].line > 0);
    }
}

#[test]
fn annotations_cover_tail_expressions_functions_and_disabled_cfg() {
    let source = r#"
        pub fn first() -> u8 {
            let x = 1u8;
            #[mirsa::check(x == 1)]
            x
        }
        pub fn second(x: u32) -> u32 {
            if x != 0 {
                #[mirsa::check(x != 0)]
                x
            } else { 1 }
        }
        #[cfg(any())]
        fn disabled() {
            #[mirsa::check(missing_name)]
            nonexistent();
        }
    "#;
    let reports = Mirsa::from_source(source).analyze_annotations().unwrap();
    assert_eq!(reports.len(), 2, "{reports:?}");
    assert!(
        reports
            .iter()
            .all(|r| matches!(&r.report.output, Output::Check(c) if c.verdict == Verdict::T))
    );
    assert!(
        Mirsa::from_source("fn f() {}")
            .analyze_annotations()
            .unwrap()
            .is_empty()
    );
}

#[test]
fn annotation_errors_do_not_silently_drop_queries() {
    for annotation in [
        "#[mirsa::check(missing > 0)]",
        "#[mirsa::check(len(x, x))]",
        "#[mirsa::check(typo(x))]",
        "#[mirsa::check()]",
        "#[mirsa::chek(x > 0)]",
    ] {
        let source = format!("pub fn f(x:usize) {{ {annotation} std::hint::black_box(x); }}");
        assert!(
            Mirsa::from_source(source).analyze_annotations().is_err(),
            "{annotation}"
        );
    }
}

#[test]
fn file_annotations_work_without_explicit_tool_registration() {
    let file = std::env::temp_dir().join(format!("mirsa-annotations-{}.rs", std::process::id()));
    std::fs::write(
        &file,
        r#"
        pub fn f() {
            let x = 5usize;
            #[mirsa::check(x < 3)]
            #[mirsa::facts(x)]
            std::hint::black_box(x);
        }
    "#,
    )
    .unwrap();
    let reports = Mirsa::from_file(&file).analyze_annotations();
    std::fs::remove_file(&file).unwrap();
    let reports = reports.unwrap();
    assert_eq!(reports.len(), 2, "{reports:?}");
    assert!(matches!(&reports[0].report.output, Output::Check(c) if c.definite_violation));
    assert_eq!(reports[0].file, file.to_string_lossy());
}

#[test]
fn nested_functions_closures_and_inactive_statements_are_scoped() {
    let code = r#"
        pub fn outer() {
            let x = 1u8;
            #[mirsa::check(x == 1)]
            std::hint::black_box(x);
            fn inner() {
                let x = 2u8;
                #[mirsa::check(x == 2)]
                std::hint::black_box(x);
            }
            let _f = || {
                let x = 3u8;
                #[mirsa::check(x == 3)]
                std::hint::black_box(x);
            };
            #[cfg(any())]
            #[mirsa::check(missing > 0)]
            nonexistent();
            inner();
        }
    "#;
    let reports = Mirsa::from_source(code).analyze_annotations().unwrap();
    assert_eq!(reports.len(), 3, "{reports:?}");
    assert!(
        reports
            .iter()
            .all(|r| matches!(&r.report.output, Output::Check(c) if c.result))
    );
    assert!(
        Mirsa::from_source("#[mirsa::check(true)] pub fn f() {}")
            .analyze_annotations()
            .is_err()
    );
}
