use super::{Error, Mirsa, Options, SourceBindings, markers::Markers};
use crate::query::{Document, Report};
use rustc_driver::{Callbacks, Compilation};
use rustc_middle::ty::TyCtxt;
use rustc_session::config::Input;
use rustc_span::FileName;
use std::path::PathBuf;

pub struct SourceInput {
    input: Input,
    rustc_args: Vec<String>,
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

    pub fn query(self, dsl: &str, bindings: &SourceBindings) -> Result<Report, Error> {
        // Parse before starting rustc, so malformed requests do not compile a crate.
        let document = crate::query::parse(dsl).map_err(Error::Query)?;
        let mut callback = QueryCallback {
            input: Some(self.input.input),
            options: self.options,
            document,
            bindings,
            result: None,
        };
        let mut args = vec![
            "rustc".into(),
            "mirsa_input.rs".into(),
            "-Zmir-opt-level=0".into(),
            "-Zmir-enable-passes=-RemoveStorageMarkers".into(),
        ];
        for default in [
            "--crate-name=mirsa_input",
            "--crate-type=lib",
            "--edition=2021",
            "--emit=metadata",
        ] {
            let (key, _) = default.split_once('=').unwrap();
            let prefix = format!("{key}=");
            if !self
                .input
                .rustc_args
                .iter()
                .any(|arg| arg == key || arg.starts_with(&prefix))
            {
                args.push(default.into());
            }
        }
        args.extend(self.input.rustc_args);
        rustc_driver::catch_fatal_errors(|| rustc_driver::run_compiler(&args, &mut callback))
            .map_err(|_| Error::Compilation)?;
        callback.result.unwrap_or(Err(Error::Compilation))
    }
}

struct QueryCallback<'a> {
    input: Option<Input>,
    options: Options,
    document: Document,
    bindings: &'a SourceBindings,
    result: Option<Result<Report, Error>>,
}

impl QueryCallback<'_> {
    fn query<'tcx>(&self, tcx: TyCtxt<'tcx>) -> Result<Report, Error> {
        let functions: Vec<_> = tcx
            .mir_keys(())
            .iter()
            .copied()
            .filter(|id| {
                matches!(
                    tcx.def_kind(*id),
                    rustc_hir::def::DefKind::Fn | rustc_hir::def::DefKind::AssocFn
                )
            })
            .collect();
        let id = Markers::find_function(tcx, functions, &self.document.statement().location)?;
        let body = tcx.optimized_mir(id.to_def_id());
        let bindings = self.bindings.resolve(tcx, body, &self.document)?;
        Mirsa::from_mir(tcx, body)
            .with_options(self.options)
            .execute(&self.document, &bindings)
    }
}

impl Callbacks for QueryCallback<'_> {
    fn config(&mut self, config: &mut rustc_interface::interface::Config) {
        config.input = self.input.take().expect("compiler configured once");
    }

    fn after_analysis<'tcx>(
        &mut self,
        _: &rustc_interface::interface::Compiler,
        tcx: TyCtxt<'tcx>,
    ) -> Compilation {
        self.result = Some(self.query(tcx));
        Compilation::Stop
    }
}
