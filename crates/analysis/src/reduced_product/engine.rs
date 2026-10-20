use super::{branch, state::CombinedState};
use crate::combined_warnings::emit_combined_warnings;
use crate::interprocedural::AnalysisSession;
use mirsa_core::cfg::Cfg;
use mirsa_domains::allocation::AllocationState;
use mirsa_domains::allocation::transfer as allocation_transfer;
use mirsa_domains::interval::IntervalState;
use mirsa_domains::interval::transfer as interval_transfer;
use mirsa_domains::nullptr::NullPtrState;
use mirsa_domains::nullptr::transfer as nullptr_transfer;
use mirsa_framework::access_path::AccessPath;
use mirsa_framework::config::{load_analysis_bool_config, load_domain_engine_config};
use mirsa_framework::forward::{
    ForwardSemantics, PathForwardAnalysisConfig, PathForwardAnalysisResult,
    state_before_location_from_result,
};
use mirsa_framework::printer::{
    print_call_pre_states, print_function_header, run_path_sensitive_analysis,
};
use mirsa_relations::symbolic as symbolic_transfer;
use rustc_hir::def_id::DefId;
use rustc_middle::mir::{BasicBlock, Body, LocalDecls, Place, Statement, Terminator};
use rustc_middle::ty::TyCtxt;

#[derive(Clone, Copy, Debug, Default)]
pub struct AnalysisOptions {
    pub debug: bool,
}

struct CombinedSemantics<'a, 'tcx> {
    tcx: TyCtxt<'tcx>,
    session: &'a AnalysisSession<'tcx>,
    entry: Option<&'a CombinedState<'tcx>>,
    slice: Option<&'a crate::query::slice::CfgSlice>,
    places: &'a [Place<'tcx>],
    all_places: &'a [Place<'tcx>],
    pointer_places: &'a [Place<'tcx>],
    interval_debug: bool,
    nullptr_debug: bool,
    allocation_debug: bool,
    symbolic_debug: bool,
    reduce_debug: bool,
}

impl<'a, 'tcx> ForwardSemantics<'tcx> for CombinedSemantics<'a, 'tcx> {
    type State = CombinedState<'tcx>;

    fn bottom(&self, body: &Body<'tcx>) -> Self::State {
        CombinedState::new_with_debug(
            IntervalState::new_bot_state(
                self.tcx,
                &body.local_decls,
                self.places,
                self.all_places,
                body.arg_count,
                self.interval_debug,
            ),
            NullPtrState::new_bot_state(self.pointer_places, body.arg_count, self.nullptr_debug),
            AllocationState::new_bottom(
                self.tcx,
                &body.local_decls,
                self.all_places,
                self.allocation_debug,
            ),
            self.symbolic_debug,
            self.reduce_debug,
        )
    }

    fn entry_state(&self, body: &Body<'tcx>) -> Self::State {
        if let Some(entry) = self.entry {
            let mut state = entry.clone();
            self.session.apply_guidance(
                self.tcx,
                body,
                None,
                rustc_middle::mir::START_BLOCK,
                &mut state,
            );
            return state;
        }
        let mut state = self.bottom(body);
        state.allocation =
            AllocationState::new_entry(self.tcx, body, self.all_places, self.allocation_debug);
        self.session.apply_guidance(
            self.tcx,
            body,
            None,
            rustc_middle::mir::START_BLOCK,
            &mut state,
        );
        state
    }

    fn transfer_block(
        &self,
        tcx: TyCtxt<'tcx>,
        body: &Body<'tcx>,
        bb: BasicBlock,
        input: &Self::State,
    ) -> Self::State {
        let mut state = input.clone();
        let data = &body.basic_blocks[bb];
        for (index, statement) in data.statements.iter().enumerate() {
            if self.slice.is_none_or(|slice| {
                slice.retains(rustc_middle::mir::Location {
                    block: bb,
                    statement_index: index,
                })
            }) {
                self.transfer_stmt(tcx, &mut state, statement, &body.local_decls);
            }
        }
        if self
            .slice
            .is_none_or(|slice| slice.transfers_terminator(bb))
        {
            self.transfer_terminator(tcx, &mut state, data.terminator(), &body.local_decls);
        }
        state
    }

    fn transfer_stmt(
        &self,
        tcx: TyCtxt<'tcx>,
        st: &mut Self::State,
        stmt: &Statement<'tcx>,
        local_decls: &LocalDecls<'tcx>,
    ) {
        transfer_stmt(tcx, st, stmt, local_decls);
    }

    fn transfer_terminator(
        &self,
        tcx: TyCtxt<'tcx>,
        st: &mut Self::State,
        term: &Terminator<'tcx>,
        local_decls: &LocalDecls<'tcx>,
    ) {
        if matches!(term.kind, rustc_middle::mir::TerminatorKind::Call { .. }) {
            st.demand_terminator(tcx, local_decls, term);
        } else {
            transfer_builtin_terminator(tcx, st, term, local_decls);
        }
    }

    fn refine_edge(
        &self,
        tcx: TyCtxt<'tcx>,
        body: &Body<'tcx>,
        pred: BasicBlock,
        succ: BasicBlock,
        in_state: &Self::State,
    ) -> Option<Self::State> {
        if self.slice.is_some_and(|slice| !slice.contains(succ)) {
            return None;
        }
        let mut state = if matches!(
            body.basic_blocks[pred].terminator().kind,
            rustc_middle::mir::TerminatorKind::Call { .. }
        ) {
            self.session.transfer_call(tcx, body, pred, succ, in_state)
        } else {
            branch::refine_edge(tcx, body, pred, succ, in_state)
        }?;
        self.session
            .apply_guidance(tcx, body, Some(pred), succ, &mut state);
        Some(state)
    }
}

pub(crate) fn transfer_builtin_terminator<'tcx>(
    tcx: TyCtxt<'tcx>,
    st: &mut CombinedState<'tcx>,
    term: &Terminator<'tcx>,
    local_decls: &LocalDecls<'tcx>,
) {
    if !st.demand_terminator(tcx, local_decls, term) {
        return;
    }
    if let Some(mirsa_framework::call_model::RangeIteratorCall::Identity(operand)) =
        mirsa_framework::call_model::RangeIteratorCall::classify(tcx, local_decls, term)
    {
        if let rustc_middle::mir::TerminatorKind::Call { destination, .. } = &term.kind {
            let statement = Statement {
                source_info: term.source_info,
                kind: rustc_middle::mir::StatementKind::Assign(Box::new((
                    *destination,
                    rustc_middle::mir::Rvalue::Use(operand),
                ))),
            };
            transfer_stmt(tcx, st, &statement, local_decls);
            return;
        }
    }
    let range_effects = mirsa_domains::interval::range_iterator::prepare_next(
        tcx,
        local_decls,
        term,
        &st.symbolic,
        &st.interval,
    );
    let written = st.symbolic.call_write_paths(tcx, term, local_decls);
    invalidate_memory(st, &written);
    symbolic_transfer::transfer_terminator(tcx, &mut st.symbolic, term, local_decls, &written);
    interval_transfer::transfer_terminator(tcx, &st.symbolic, &mut st.interval, term, local_decls);
    nullptr_transfer::transfer_terminator(tcx, &st.symbolic, &mut st.nullptr, term, local_decls);
    allocation_transfer::transfer_terminator(
        tcx,
        &st.symbolic,
        &mut st.allocation,
        term,
        local_decls,
        |operand| {
            interval_transfer::eval_operand(
                tcx,
                local_decls,
                operand,
                &st.symbolic,
                &mut st.interval,
            )
        },
    );
    if let Some(effects) = range_effects {
        effects.apply(&mut st.interval);
    }
    st.reduce_call_result(tcx, local_decls, term);
}

pub(crate) fn invalidate_memory(st: &mut CombinedState<'_>, written: &[AccessPath]) {
    st.record_writes(written.iter().cloned());
    st.interval.invalidate_paths(&st.symbolic, written);
    st.nullptr.invalidate_paths(&st.symbolic, written);
    st.allocation.invalidate_paths(&st.symbolic, written);
    for path in written {
        st.symbolic.kill_path_tree(path);
    }
}

struct IndexResolver<'a, 'tcx> {
    tcx: TyCtxt<'tcx>,
    state: &'a CombinedState<'tcx>,
    unresolved: bool,
}

impl<'tcx> rustc_middle::mir::visit::MutVisitor<'tcx> for IndexResolver<'_, 'tcx> {
    fn tcx(&self) -> TyCtxt<'tcx> {
        self.tcx
    }

    fn visit_place(
        &mut self,
        place: &mut Place<'tcx>,
        _context: rustc_middle::mir::visit::PlaceContext,
        _location: rustc_middle::mir::Location,
    ) {
        if let Some(resolved) = super::demand::resolve_place(self.tcx, self.state, *place) {
            *place = resolved;
        } else {
            self.unresolved = true;
        }
    }
}

pub(crate) fn transfer_stmt<'tcx>(
    tcx: TyCtxt<'tcx>,
    st: &mut CombinedState<'tcx>,
    stmt: &Statement<'tcx>,
    local_decls: &LocalDecls<'tcx>,
) {
    if !st.demand_statement(tcx, local_decls, stmt) {
        return;
    }
    let mut resolved = stmt.clone();
    let mut resolver = IndexResolver {
        tcx,
        state: st,
        unresolved: false,
    };
    rustc_middle::mir::visit::MutVisitor::visit_statement(
        &mut resolver,
        &mut resolved,
        rustc_middle::mir::Location {
            block: rustc_middle::mir::START_BLOCK,
            statement_index: 0,
        },
    );
    if resolver.unresolved {
        let mut written = st.symbolic.indirect_write_paths(&resolved);
        if let rustc_middle::mir::StatementKind::Assign(assign) = &resolved.kind {
            if let Some(path) = st.symbolic.normalize_place(assign.0) {
                written.push(path);
            }
        }
        invalidate_memory(st, &written);
        return;
    }
    let stmt = &resolved;
    if let rustc_middle::mir::StatementKind::Assign(assign) = &stmt.kind {
        st.record_writes(st.symbolic.normalize_place(assign.0));
    }
    if let rustc_middle::mir::StatementKind::SetDiscriminant { place, .. }
    | rustc_middle::mir::StatementKind::Deinit(place) = &stmt.kind
    {
        let paths: Vec<_> = st.symbolic.normalize_place(**place).into_iter().collect();
        invalidate_memory(st, &paths);
    }
    let written = st.symbolic.indirect_write_paths(stmt);
    invalidate_memory(st, &written);
    symbolic_transfer::transfer_stmt(tcx, &mut st.symbolic, stmt, local_decls);
    interval_transfer::transfer_stmt(tcx, &st.symbolic, &mut st.interval, stmt, local_decls);
    nullptr_transfer::transfer_stmt(tcx, &st.symbolic, &mut st.nullptr, stmt, local_decls);
    allocation_transfer::transfer_stmt(
        tcx,
        &st.symbolic,
        &mut st.allocation,
        stmt,
        local_decls,
        |operand| {
            interval_transfer::eval_operand(
                tcx,
                local_decls,
                operand,
                &st.symbolic,
                &mut st.interval,
            )
        },
    );
}

pub fn state_before_location<'tcx>(
    tcx: TyCtxt<'tcx>,
    body: &Body<'tcx>,
    result: &PathForwardAnalysisResult<CombinedState<'tcx>>,
    location: rustc_middle::mir::Location,
) -> Option<CombinedState<'tcx>> {
    let mut state = state_before_location_from_result(tcx, body, result, location, transfer_stmt)?;
    let block = &body.basic_blocks[location.block];
    let feasible = if let Some(statement) = block.statements.get(location.statement_index) {
        state.demand_statement(tcx, &body.local_decls, statement)
    } else if let Some(term) = &block.terminator {
        state.demand_terminator(tcx, &body.local_decls, term)
    } else {
        true
    };
    feasible.then_some(state)
}

fn print_unsafe_pre_states<'tcx>(
    tcx: TyCtxt<'tcx>,
    body: &Body<'tcx>,
    result: &PathForwardAnalysisResult<CombinedState<'tcx>>,
) {
    print_call_pre_states(
        tcx,
        body,
        result,
        state_before_location,
        |tcx, body, term| mirsa_contracts::matcher::classify_call(tcx, body, term).is_some(),
    );
}

pub fn run_combined<'tcx>(
    tcx: TyCtxt<'tcx>,
    def_id: DefId,
    body: &Body<'tcx>,
    cfg: &Cfg,
    places: &[Place<'tcx>],
    all_places: &[Place<'tcx>],
    pointer_places: &[Place<'tcx>],
    options: AnalysisOptions,
) {
    let interval_config = load_domain_engine_config("interval");
    let nullptr_config = load_domain_engine_config("nullptr");
    let allocation_config = load_domain_engine_config("allocation");
    let warn_on_maybe = load_analysis_bool_config("warn_on_maybe", false);

    let max_iterations = [
        interval_config.max_iterations,
        nullptr_config.max_iterations,
        allocation_config.max_iterations,
    ]
    .into_iter()
    .flatten()
    .min();
    let result = analyze_combined_with_config(
        tcx,
        body,
        cfg,
        places,
        all_places,
        pointer_places,
        PathForwardAnalysisConfig {
            max_paths: interval_config
                .max_paths
                .max(nullptr_config.max_paths)
                .max(allocation_config.max_paths),
            widen_after_iterations: max_iterations,
            max_duration: None,
        },
        options,
    );

    print_function_header(tcx, def_id);
    print_unsafe_pre_states(tcx, body, &result);

    emit_combined_warnings(tcx, body, &result, warn_on_maybe);
}

pub fn analyze_combined_with_config<'tcx>(
    tcx: TyCtxt<'tcx>,
    body: &Body<'tcx>,
    cfg: &Cfg,
    places: &[Place<'tcx>],
    all_places: &[Place<'tcx>],
    pointer_places: &[Place<'tcx>],
    analysis_config: PathForwardAnalysisConfig,
    options: AnalysisOptions,
) -> PathForwardAnalysisResult<CombinedState<'tcx>> {
    let session = AnalysisSession::new(analysis_config);
    analyze_with_entry(
        tcx,
        body,
        cfg,
        places,
        all_places,
        pointer_places,
        analysis_config,
        options,
        &session,
        None,
        None,
    )
}

pub(crate) fn analyze_with_entry<'tcx>(
    tcx: TyCtxt<'tcx>,
    body: &Body<'tcx>,
    cfg: &Cfg,
    places: &[Place<'tcx>],
    all_places: &[Place<'tcx>],
    pointer_places: &[Place<'tcx>],
    analysis_config: PathForwardAnalysisConfig,
    options: AnalysisOptions,
    session: &AnalysisSession<'tcx>,
    entry: Option<&CombinedState<'tcx>>,
    slice: Option<&crate::query::slice::CfgSlice>,
) -> PathForwardAnalysisResult<CombinedState<'tcx>> {
    let debug = options.debug;
    let semantics = CombinedSemantics {
        tcx,
        session,
        entry,
        slice,
        places,
        all_places,
        pointer_places,
        interval_debug: debug,
        nullptr_debug: debug,
        allocation_debug: debug,
        symbolic_debug: debug,
        reduce_debug: debug,
    };
    run_path_sensitive_analysis(tcx, body, cfg, &semantics, analysis_config)
}
