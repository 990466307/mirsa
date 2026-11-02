use super::Mirsa;
use rustc_session::config::Input;
use rustc_span::FileName;
use std::path::PathBuf;

pub struct SourceInput {
    pub(super) input: Input,
    pub(super) rustc_args: Vec<String>,
}

impl SourceInput {
    pub(super) fn text(source: String) -> Self {
        Self {
            input: Input::Str {
                name: FileName::Custom("mirsa_input.rs".into()),
                input: source,
            },
            rustc_args: Vec::new(),
        }
    }

    pub(super) fn file(path: PathBuf) -> Self {
        Self {
            input: Input::File(path),
            rustc_args: Vec::new(),
        }
    }
}

impl Mirsa<SourceInput> {
    /// Extra compiler flags, e.g. `--extern`, `-L`, `--cfg`, or `--sysroot`.
    pub fn rustc_args(mut self, args: impl IntoIterator<Item = String>) -> Self {
        self.input.rustc_args.extend(args);
        self
    }
}

/// Shared driver flags: source attributes are inert tool metadata, not proc macros.
pub(super) fn compiler_args(rustc_args: Vec<String>) -> Vec<String> {
    let mut args = vec![
        "rustc".into(),
        "mirsa_input.rs".into(),
        "-Zmir-opt-level=0".into(),
        "-Zmir-enable-passes=-RemoveStorageMarkers".into(),
        "-Zcrate-attr=feature(stmt_expr_attributes)".into(),
    ];
    for default in [
        "--crate-name=mirsa_input",
        "--crate-type=lib",
        "--edition=2021",
        "--emit=metadata",
    ] {
        let (key, _) = default.split_once('=').unwrap();
        let prefix = format!("{key}=");
        if !rustc_args
            .iter()
            .any(|arg| arg == key || arg.starts_with(&prefix))
        {
            args.push(default.into());
        }
    }
    args.extend(rustc_args);
    args
}
