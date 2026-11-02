mod abstract_query;
mod demand;
pub(crate) mod slice;
pub use slice::SliceStats;
mod bind;
mod facts;
pub(crate) mod guidance;
pub use guidance::{Candidate, GuidanceCheck};
mod output;
mod value;
pub use bind::{Bindings, ValueBinding};
pub use facts::model::{
    AbstractFact, BinaryOperator, Expression, FactOutput, PathFacts, Projection, Property,
    SymbolicFact, UnaryOperator,
};
pub use mirsa_dsl::{Diagnostic, DiagnosticContext, Document, GRAMMAR, parse, parse_expression};
pub use output::{FactValue, Output, PointerTarget, Report};
mod encode;
mod evidence;
mod property;
mod result;
pub use evidence::{
    Evaluation, EvaluationRule, Evidence, ExpressionEvidence, SolverBinding, SolverEvidence,
};
pub use result::{CheckReport, PathProof, Proof, ProofSource, Status, Verdict};

use crate::reduced_product::{CombinedState, engine::transfer_stmt};
use mirsa_framework::forward::{PathForwardAnalysisResult, replay_state_before_location};
pub use mirsa_solver::UnknownReason;
use mirsa_solver::Verdict as SolverVerdict;
use property::{BinaryOp, Comparison, Expr, Predicate, UnaryOp};
use rustc_middle::{
    mir::{Body, Location},
    ty::TyCtxt,
};
use std::borrow::Cow;
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug)]
pub struct QueryConfig {
    pub use_solver: bool,
    pub max_depth: usize,
    pub max_values: usize,
    pub max_nodes: usize,
    pub solver_timeout: Duration,
    pub total_timeout: Duration,
}

impl Default for QueryConfig {
    fn default() -> Self {
        Self {
            use_solver: cfg!(feature = "z3"),
            max_depth: 8,
            max_values: 64,
            max_nodes: 1024,
            solver_timeout: Duration::from_millis(100),
            total_timeout: Duration::from_secs(1),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QueryError {
    IncompleteAnalysis,
    InvalidLocation,
    BudgetExceeded,
    Unsupported,
}

pub struct AnalysisView<'a, 'tcx> {
    tcx: TyCtxt<'tcx>,
    body: &'a Body<'tcx>,
    result: Option<Cow<'a, PathForwardAnalysisResult<CombinedState<'tcx>>>>,
    session: Option<&'a crate::interprocedural::AnalysisSession<'tcx>>,
    slice: Option<slice::CfgSlice>,
    config: QueryConfig,
}

impl<'a, 'tcx> AnalysisView<'a, 'tcx> {
    pub fn new(
        tcx: TyCtxt<'tcx>,
        body: &'a Body<'tcx>,
        result: &'a PathForwardAnalysisResult<CombinedState<'tcx>>,
    ) -> Self {
        Self {
            tcx,
            body,
            result: Some(Cow::Borrowed(result)),
            session: None,
            slice: None,
            config: QueryConfig::default(),
        }
    }

    pub(crate) fn from_owned(
        tcx: TyCtxt<'tcx>,
        body: &'a Body<'tcx>,
        result: PathForwardAnalysisResult<CombinedState<'tcx>>,
    ) -> Self {
        Self {
            tcx,
            body,
            result: Some(Cow::Owned(result)),
            session: None,
            slice: None,
            config: QueryConfig::default(),
        }
    }

    pub(crate) fn from_session(
        tcx: TyCtxt<'tcx>,
        body: &'a Body<'tcx>,
        session: &'a crate::interprocedural::AnalysisSession<'tcx>,
    ) -> Self {
        Self {
            tcx,
            body,
            result: None,
            session: Some(session),
            slice: None,
            config: QueryConfig::default(),
        }
    }

    fn prepare(
        &self,
        targets: &[Location],
        expressions: &[&Expr<'tcx>],
        complete: bool,
    ) -> Result<Option<Self>, QueryError> {
        if targets.iter().any(|at| {
            at.block.index() >= self.body.basic_blocks.len()
                || at.statement_index > self.body.basic_blocks[at.block].statements.len()
        }) {
            return Err(QueryError::InvalidLocation);
        }
        let Some(session) = self.session else {
            return Ok(None);
        };
        let mut demand = demand::Demand::default();
        for expression in expressions {
            demand.expression(expression);
        }
        let slice = (!complete).then(|| slice::CfgSlice::new(self.body, targets, demand));
        let result = match &slice {
            Some(slice) => session.analyze_slice(self.tcx, self.body, slice),
            None => session.analyze(self.tcx, self.body),
        };
        let mut view = Self::from_owned(self.tcx, self.body, result).with_config(self.config);
        if session.slice_stats().is_some() {
            view.slice = slice;
        }
        Ok(Some(view))
    }

    pub fn with_config(mut self, config: QueryConfig) -> Self {
        self.config = config;
        self
    }

    fn at(&self, location: Location) -> Result<QueryView<'a, 'tcx>, QueryError> {
        let result = self.result.as_ref().expect("query prepared before replay");
        if !result.completed {
            return Err(QueryError::IncompleteAnalysis);
        }
        if location.block.index() >= self.body.basic_blocks.len()
            || location.statement_index > self.body.basic_blocks[location.block].statements.len()
        {
            return Err(QueryError::InvalidLocation);
        }
        let mut states = Vec::new();
        for path in &result.paths {
            if !path
                .reachable
                .get(location.block.index())
                .copied()
                .unwrap_or(false)
            {
                continue;
            }
            let Some(entry) = path.in_states.get(location.block.index()) else {
                return Err(QueryError::IncompleteAnalysis);
            };
            let state = if let Some(slice) = &self.slice {
                let mut state = entry.clone();
                for index in 0..location.statement_index {
                    if slice.retains(Location {
                        block: location.block,
                        statement_index: index,
                    }) {
                        transfer_stmt(
                            self.tcx,
                            &mut state,
                            &self.body.basic_blocks[location.block].statements[index],
                            &self.body.local_decls,
                        );
                    }
                }
                state
            } else {
                replay_state_before_location(self.tcx, self.body, entry, location, transfer_stmt)
                    .ok_or(QueryError::InvalidLocation)?
            };
            if state.feasible {
                states.push((path.id, path.merged, state));
            }
        }
        Ok(QueryView {
            tcx: self.tcx,
            body: self.body,
            states,
            config: self.config,
        })
    }
}

struct QueryView<'a, 'tcx> {
    tcx: TyCtxt<'tcx>,
    body: &'a Body<'tcx>,
    states: Vec<(u32, bool, CombinedState<'tcx>)>,
    config: QueryConfig,
}

impl<'tcx> QueryView<'_, 'tcx> {
    fn prove(
        &mut self,
        property: &Predicate<'tcx>,
        bindings: &Bindings<'tcx>,
    ) -> Result<Proof, QueryError> {
        let started = Instant::now();
        if self.states.is_empty() {
            return Ok(Proof::from_paths(Vec::new()));
        }
        if !property.within_budget(self.config.max_nodes) {
            return Err(QueryError::BudgetExceeded);
        }
        // Finish every path's abstract evaluation before optional SMT refinement.
        // A solver timeout must not prevent later paths from retaining evidence.
        let mut evaluated = Vec::new();
        for (_, _, state) in &mut self.states {
            if started.elapsed() >= self.config.total_timeout {
                return Err(QueryError::BudgetExceeded);
            }
            value::reduce(self.tcx, self.body, state, property);
            let result = abstract_query::prove(self.tcx, self.body, state, property);
            let evidence = evidence::collect(self.tcx, self.body, state, property, bindings);
            if started.elapsed() >= self.config.total_timeout {
                return Err(QueryError::BudgetExceeded);
            }
            evaluated.push((result, evidence));
        }
        let mut paths = Vec::new();
        for ((id, merged, state), (mut result, mut evidence)) in self.states.iter().zip(evaluated) {
            let mut source = ProofSource::Abstract;
            let mut notes = Vec::new();
            if result.verdict == SolverVerdict::Unknown && self.config.use_solver {
                if started.elapsed() >= self.config.total_timeout {
                    notes.push(solver_fallback_note(UnknownReason::BudgetExceeded).into());
                } else {
                    match encode::problem(
                        self.tcx,
                        self.body,
                        state,
                        property,
                        &self.config,
                        bindings,
                    ) {
                        Ok((problem, solver_bindings)) => {
                            let solved = mirsa_solver::solve(
                                &problem,
                                self.config
                                    .total_timeout
                                    .saturating_sub(started.elapsed())
                                    .min(self.config.solver_timeout),
                            );
                            if let Some(reason) = solved
                                .reason
                                .filter(|reason| *reason != UnknownReason::InsufficientInformation)
                            {
                                if reason == UnknownReason::Unsupported {
                                    return Err(QueryError::Unsupported);
                                }
                                notes.push(solver_fallback_note(reason).into());
                            } else {
                                source = ProofSource::Solver;
                                evidence.smt = Some(SolverEvidence {
                                    problem,
                                    bindings: solver_bindings,
                                    checks: solved.evidence.clone(),
                                });
                                result = solved;
                            }
                        }
                        Err(UnknownReason::Unsupported) => return Err(QueryError::Unsupported),
                        Err(reason) => notes.push(solver_fallback_note(reason).into()),
                    }
                }
            }
            let mut path = PathProof::new(*id, *merged, source, result);
            path.notes = notes;
            evidence.expressions = evidence.expression_view();
            path.evidence = evidence;
            paths.push(path);
        }
        Ok(Proof::from_paths(paths))
    }
}

fn solver_fallback_note(reason: UnknownReason) -> &'static str {
    match reason {
        UnknownReason::BudgetExceeded => {
            "Z3 补充求解超时或超过预算，未采用其结果；本路径使用抽象分析结论和证据。"
        }
        UnknownReason::BackendDisabled => "Z3 未启用；本路径使用抽象分析结论和证据。",
        UnknownReason::SolverUnknown => {
            "Z3 未完成判定，未采用其结果；本路径使用抽象分析结论和证据。"
        }
        UnknownReason::Unsupported | UnknownReason::InsufficientInformation => unreachable!(),
    }
}
