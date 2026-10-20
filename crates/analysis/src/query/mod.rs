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
    AbstractFact, BinaryOperator, Expression, FactOutput, Projection, Property, SymbolicFact,
    UnaryOperator,
};
pub use mirsa_dsl::{Diagnostic, Document, GRAMMAR, parse, parse_expression};
pub use output::{FactValue, Output, PointerTarget, Report};
mod encode;
mod property;
mod result;
pub use result::{CheckReport, PathProof, Proof, ProofSource, Verdict};

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
    fn prove(&mut self, property: &Predicate<'tcx>) -> Proof {
        let started = Instant::now();
        if !property.within_budget(self.config.max_nodes) {
            return Proof {
                verdict: Verdict::F,
                definite_violation: false,
                unreachable: self.states.is_empty(),
                reason: Some(UnknownReason::BudgetExceeded),
                paths: Vec::new(),
            };
        }
        let mut paths = Vec::new();
        for (id, merged, state) in &mut self.states {
            let mut source = ProofSource::Abstract;
            let mut result = if started.elapsed() >= self.config.total_timeout {
                mirsa_solver::SolveResult::unknown(UnknownReason::BudgetExceeded)
            } else {
                value::reduce(self.tcx, self.body, state, property);
                abstract_query::prove(self.tcx, self.body, state, property)
            };
            if result.verdict == SolverVerdict::Unknown && self.config.use_solver {
                let remaining = self.config.total_timeout.saturating_sub(started.elapsed());
                if remaining.is_zero() {
                    result.reason = Some(UnknownReason::BudgetExceeded);
                } else {
                    source = ProofSource::Solver;
                    result =
                        match encode::problem(self.tcx, self.body, state, property, &self.config) {
                            Ok(problem) => mirsa_solver::solve(
                                &problem,
                                self.config
                                    .total_timeout
                                    .saturating_sub(started.elapsed())
                                    .min(self.config.solver_timeout),
                            ),
                            Err(reason) => mirsa_solver::SolveResult::unknown(reason),
                        };
                }
            }
            paths.push(PathProof::new(*id, *merged, source, result));
        }
        Proof::from_paths(paths)
    }
}
