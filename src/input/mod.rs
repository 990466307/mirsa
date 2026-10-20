mod bindings;
mod markers;
mod source;

pub use bindings::SourceBindings;
pub use source::SourceInput;

use crate::analysis::interprocedural::AnalysisSession;
use crate::framework::forward::PathForwardAnalysisConfig;
use crate::query::{Bindings, Diagnostic, Document, QueryConfig, Report};
use rustc_middle::{mir::Body, ty::TyCtxt};
use std::{fmt, path::PathBuf, time::Duration};

/// Shared configuration for both source and MIR inputs.
#[derive(Clone, Copy, Debug)]
pub struct Options {
    pub analysis: PathForwardAnalysisConfig,
    pub query: QueryConfig,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            analysis: PathForwardAnalysisConfig {
                max_paths: 8,
                widen_after_iterations: Some(8),
                max_duration: Some(Duration::from_secs(30)),
            },
            query: QueryConfig::default(),
        }
    }
}

#[derive(Debug)]
pub enum Error {
    /// rustc has already emitted the compilation diagnostics.
    Compilation,
    Binding(String),
    Query(Vec<Diagnostic>),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Compilation => f.write_str("源码编译失败，详见编译器诊断"),
            Self::Binding(message) => f.write_str(message),
            Self::Query(errors) => {
                for error in errors {
                    writeln!(f, "DSL {:?}: {}", error.span, error.message)?;
                }
                Ok(())
            }
        }
    }
}
impl std::error::Error for Error {}

/// A single query: supply a program, bindings and DSL; defaults handle analysis.
pub struct Mirsa<I = ()> {
    input: I,
    options: Options,
}

pub struct MirInput<'a, 'tcx> {
    tcx: TyCtxt<'tcx>,
    body: &'a Body<'tcx>,
}

impl Mirsa {
    pub fn from_source(source: impl Into<String>) -> Mirsa<SourceInput> {
        Mirsa::new(SourceInput::text(source.into()))
    }

    pub fn from_file(path: impl Into<PathBuf>) -> Mirsa<SourceInput> {
        Mirsa::new(SourceInput::file(path.into()))
    }

    pub fn from_mir<'a, 'tcx>(
        tcx: TyCtxt<'tcx>,
        body: &'a Body<'tcx>,
    ) -> Mirsa<MirInput<'a, 'tcx>> {
        Mirsa::new(MirInput { tcx, body })
    }
}

impl<I> Mirsa<I> {
    fn new(input: I) -> Self {
        Self {
            input,
            options: Options::default(),
        }
    }

    pub fn with_options(mut self, options: Options) -> Self {
        self.options = options;
        self
    }
}

impl<'tcx> Mirsa<MirInput<'_, 'tcx>> {
    pub fn query(self, dsl: &str, bindings: &Bindings<'tcx>) -> Result<Report, Error> {
        let document = crate::query::parse(dsl).map_err(Error::Query)?;
        self.execute(&document, bindings)
    }

    pub fn execute(self, document: &Document, bindings: &Bindings<'tcx>) -> Result<Report, Error> {
        let session = AnalysisSession::new(self.options.analysis);
        session
            .analyze_for_queries(self.input.tcx, self.input.body)
            .with_config(self.options.query)
            .execute(document, bindings)
            .map_err(Error::Query)
    }
}
