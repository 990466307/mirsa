//! Source tool attributes, collected from HIR after cfg and macro expansion.
use super::{Error, Mirsa, Options, SourceInput, bindings, location};
use crate::analysis::interprocedural::AnalysisSession;
use crate::query::Report;
use mirsa_dsl::RequestKind;
use rustc_driver::{Callbacks, Compilation};
use rustc_hir::{
    self as hir,
    intravisit::{self, Visitor},
};
use rustc_middle::ty::TyCtxt;
use rustc_session::config::Input;
use rustc_span::{Span, Symbol, symbol::Ident};
use std::collections::HashSet;

#[derive(Clone, Debug)]
#[cfg_attr(feature = "cli", derive(serde::Serialize))]
pub struct AnnotationReport {
    pub file: String,
    pub line: usize,
    pub column: usize,
    pub query: String,
    pub report: Report,
}

pub(super) fn register(config: &mut rustc_interface::interface::Config) {
    config.override_queries = Some(|_, providers| {
        providers.registered_tools = |tcx, ()| {
            let mut tools = (rustc_interface::DEFAULT_QUERY_PROVIDERS.registered_tools)(tcx, ());
            tools.insert(Ident::from_str("mirsa"));
            tools
        };
    });
}

impl Mirsa<SourceInput> {
    /// Check all #[mirsa::check(...)] and #[mirsa::facts(...)] statement attributes.
    pub fn analyze_annotations(self) -> Result<Vec<AnnotationReport>, Error> {
        let mut callback = AnnotationCallback {
            input: Some(self.input.input),
            options: self.options,
            result: None,
        };
        let args = super::source::compiler_args(self.input.rustc_args);
        rustc_driver::catch_fatal_errors(|| rustc_driver::run_compiler(&args, &mut callback))
            .map_err(|_| Error::Compilation)?;
        callback.result.unwrap_or(Err(Error::Compilation))
    }
}

struct Annotation {
    span: Span,
    target: Span,
    kind: RequestKind,
    source: String,
}
struct Collector<'tcx> {
    tcx: TyCtxt<'tcx>,
    found: Vec<Annotation>,
    seen: HashSet<Span>,
    error: Option<Error>,
}
impl Collector<'_> {
    fn attrs(&mut self, id: hir::HirId, target: Span) {
        for attr in self.tcx.hir().attrs(id) {
            let hir::AttrKind::Normal(item) = &attr.kind else {
                continue;
            };
            let segments = &item.path.segments;
            if segments
                .first()
                .is_none_or(|s| s.name != Symbol::intern("mirsa"))
            {
                continue;
            }
            if !self.seen.insert(attr.span) {
                continue;
            }
            if attr.span.from_expansion() {
                self.error = Some(Error::Binding(
                    "mirsa 属性暂不支持宏生成的位置；请标注宏外的语句".into(),
                ));
                continue;
            }
            let kind = match segments.as_ref() {
                [_, op] if op.name == Symbol::intern("check") => RequestKind::Check,
                [_, op] if op.name == Symbol::intern("facts") => RequestKind::Facts,
                _ => {
                    self.error = Some(Error::Binding(
                        "未知 mirsa 属性；使用 #[mirsa::check(...)] 或 #[mirsa::facts(...)]".into(),
                    ));
                    continue;
                }
            };
            let snippet = self
                .tcx
                .sess
                .source_map()
                .span_to_snippet(attr.span)
                .unwrap_or_default();
            let payload = snippet
                .find('(')
                .zip(snippet.rfind(')'))
                .filter(|(a, b)| a < b);
            if let Some((a, b)) = payload {
                self.found.push(Annotation {
                    span: attr.span,
                    target,
                    kind,
                    source: snippet[a + 1..b].into(),
                });
            } else {
                self.error = Some(Error::Binding("mirsa 属性需要括号和查询表达式".into()));
            }
        }
    }
}
impl<'tcx> Visitor<'tcx> for Collector<'tcx> {
    fn visit_stmt(&mut self, stmt: &'tcx hir::Stmt<'tcx>) {
        self.attrs(stmt.hir_id, stmt.span);
        intravisit::walk_stmt(self, stmt);
    }
    fn visit_expr(&mut self, expr: &'tcx hir::Expr<'tcx>) {
        self.attrs(expr.hir_id, expr.span);
        intravisit::walk_expr(self, expr);
    }
}
struct AnnotationCallback {
    input: Option<Input>,
    options: Options,
    result: Option<Result<Vec<AnnotationReport>, Error>>,
}
impl AnnotationCallback {
    fn analyze(&self, tcx: TyCtxt<'_>) -> Result<Vec<AnnotationReport>, Error> {
        let mut reports = Vec::new();
        for id in tcx.mir_keys(()) {
            let Some(body_id) = tcx.hir().maybe_body_owned_by(*id) else {
                continue;
            };
            if tcx.hir().attrs(tcx.local_def_id_to_hir_id(*id)).iter().any(|attr| {
                matches!(&attr.kind, hir::AttrKind::Normal(item) if item.path.segments.first().is_some_and(|s| s.name == Symbol::intern("mirsa")))
            }) {
                return Err(Error::Binding("mirsa 查询属性应标在语句或表达式上，不能标在函数上".into()));
            }
            let mut collector = Collector {
                tcx,
                found: vec![],
                seen: HashSet::new(),
                error: None,
            };
            collector.visit_body(body_id);
            if let Some(error) = collector.error {
                return Err(error);
            }
            if collector.found.is_empty() {
                continue;
            }
            let body = tcx.optimized_mir(id.to_def_id());
            let session = AnalysisSession::new(self.options.analysis);
            let view = session
                .analyze_for_queries(tcx, body)
                .with_config(self.options.query);
            for (i, annotation) in collector.found.into_iter().enumerate() {
                let document = mirsa_dsl::parse_annotation(
                    &annotation.source,
                    annotation.kind,
                    &format!("attribute_{i}"),
                )
                .map_err(Error::Query)?;
                let point = location::at_span(
                    body,
                    annotation.target,
                    annotation.span.lo(),
                    document.statement().location.as_str(),
                )?;
                let bindings = bindings::resolve_at(body, &document, point)?;
                let report = view.execute(&document, &bindings).map_err(Error::Query)?;
                let loc = tcx.sess.source_map().lookup_char_pos(annotation.span.lo());
                reports.push(AnnotationReport {
                    file: loc.file.name.prefer_local().to_string(),
                    line: loc.line,
                    column: loc.col.0 + 1,
                    query: annotation.source,
                    report,
                });
            }
        }
        reports.sort_by(|a, b| (&a.file, a.line, a.column).cmp(&(&b.file, b.line, b.column)));
        Ok(reports)
    }
}
impl Callbacks for AnnotationCallback {
    fn config(&mut self, config: &mut rustc_interface::interface::Config) {
        config.input = self.input.take().expect("configured once");
        register(config);
    }
    fn after_analysis<'tcx>(
        &mut self,
        _: &rustc_interface::interface::Compiler,
        tcx: TyCtxt<'tcx>,
    ) -> Compilation {
        self.result = Some(self.analyze(tcx));
        Compilation::Stop
    }
}
