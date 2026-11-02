#![feature(rustc_private)]

use mirsa::{Mirsa, query::Output};
use std::path::PathBuf;

struct TestCrate(PathBuf);

impl Drop for TestCrate {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn crate_root_collects_queries_from_external_and_nested_modules() {
    let project =
        TestCrate(std::env::temp_dir().join(format!("mirsa-crate-modules-{}", std::process::id())));
    std::fs::create_dir_all(project.0.join("nested")).unwrap();
    std::fs::write(
        project.0.join("lib.rs"),
        "pub mod checked; pub mod nested; #[cfg(any())] mod absent;",
    )
    .unwrap();
    std::fs::write(
        project.0.join("checked.rs"),
        r#"pub fn in_bounds() -> u8 {
    let data = [1u8, 2, 3];
    let index = 1usize;
    #[mirsa::check(index < len(data))]
    unsafe { *data.get_unchecked(index) }
}
"#,
    )
    .unwrap();
    std::fs::write(project.0.join("nested/mod.rs"), "pub mod invalid;").unwrap();
    std::fs::write(
        project.0.join("nested/invalid.rs"),
        r#"pub fn out_of_bounds() -> u8 {
    let data = [1u8, 2, 3];
    let index = 5usize;
    #[mirsa::check(index < len(data))]
    unsafe { *data.get_unchecked(index) }
}
"#,
    )
    .unwrap();

    let reports = Mirsa::from_file(project.0.join("lib.rs"))
        .analyze_annotations()
        .unwrap();
    assert_eq!(reports.len(), 2, "{reports:?}");
    for (file, line, proved) in [("checked.rs", 4, true), ("nested/invalid.rs", 4, false)] {
        let expected_path = project.0.join(file).canonicalize().unwrap();
        let report = reports
            .iter()
            .find(|report| PathBuf::from(&report.file).canonicalize().unwrap() == expected_path)
            .unwrap_or_else(|| panic!("missing report for {file}: {reports:?}"));
        assert_eq!(report.line, line);
        let Output::Check(check) = &report.report.output else {
            panic!("expected a property check: {report:?}");
        };
        assert_eq!(check.result, proved, "{file}: {check:?}");
        assert_eq!(check.definite_violation, !proved, "{file}: {check:?}");
    }
}
