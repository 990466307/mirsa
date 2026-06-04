use super::{branch, state::CombinedState};
use crate::combined_warnings::emit_combined_warnings;
use mirsa_core::cfg::Cfg;
use mirsa_domains::interval::IntervalState;
use mirsa_domains::interval::transfer as interval_transfer;
use mirsa_domains::nullptr::NullPtrState;
use mirsa_domains::nullptr::transfer as nullptr_transfer;
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
    places: &'a [Place<'tcx>],
    all_places: &'a [Place<'tcx>],
    pointer_places: &'a [Place<'tcx>],
    interval_debug: bool,
    nullptr_debug: bool,
    symbolic_debug: bool,
    reduce_debug: bool,
}

impl<'a, 'tcx> ForwardSemantics<'tcx> for CombinedSemantics<'a, 'tcx> {
    type State = CombinedState<'tcx>;

    fn bottom(&self, body: &Body<'tcx>) -> Self::State {
        CombinedState::new_with_debug(
            IntervalState::new_bot_state(
                self.places,
                self.all_places,
                body.arg_count,
                self.interval_debug,
            ),
            NullPtrState::new_bot_state(self.pointer_places, body.arg_count, self.nullptr_debug),
            self.symbolic_debug,
            self.reduce_debug,
        )
    }

    fn transfer_stmt(
        &self,
        tcx: TyCtxt<'tcx>,
        st: &mut Self::State,
        stmt: &Statement<'tcx>,
        local_decls: &LocalDecls<'tcx>,
    ) {
        symbolic_transfer::transfer_stmt(tcx, &mut st.symbolic, stmt, local_decls);
        interval_transfer::transfer_stmt(tcx, &st.symbolic, &mut st.interval, stmt, local_decls);
        nullptr_transfer::transfer_stmt(tcx, &st.symbolic, &mut st.nullptr, stmt, local_decls);
        let _ = st.refine_with_path_facts(tcx, local_decls);
    }

    fn transfer_terminator(
        &self,
        tcx: TyCtxt<'tcx>,
        st: &mut Self::State,
        term: &Terminator<'tcx>,
        local_decls: &LocalDecls<'tcx>,
    ) {
        symbolic_transfer::transfer_terminator(tcx, &mut st.symbolic, term, local_decls);
        interval_transfer::transfer_terminator(
            tcx,
            &st.symbolic,
            &mut st.interval,
            term,
            local_decls,
        );
        nullptr_transfer::transfer_terminator(
            tcx,
            &st.symbolic,
            &mut st.nullptr,
            term,
            local_decls,
        );
        let _ = st.refine_with_path_facts(tcx, local_decls);
    }

    fn refine_edge(
        &self,
        tcx: TyCtxt<'tcx>,
        body: &Body<'tcx>,
        pred: BasicBlock,
        succ: BasicBlock,
        in_state: &Self::State,
    ) -> Option<Self::State> {
        branch::refine_edge(tcx, body, pred, succ, in_state)
    }
}

fn transfer_stmt<'tcx>(
    tcx: TyCtxt<'tcx>,
    st: &mut CombinedState<'tcx>,
    stmt: &Statement<'tcx>,
    local_decls: &LocalDecls<'tcx>,
) {
    symbolic_transfer::transfer_stmt(tcx, &mut st.symbolic, stmt, local_decls);
    interval_transfer::transfer_stmt(tcx, &st.symbolic, &mut st.interval, stmt, local_decls);
    nullptr_transfer::transfer_stmt(tcx, &st.symbolic, &mut st.nullptr, stmt, local_decls);
    let _ = st.refine_with_path_facts(tcx, local_decls);
}

pub fn state_before_location<'tcx>(
    tcx: TyCtxt<'tcx>,
    body: &Body<'tcx>,
    result: &PathForwardAnalysisResult<CombinedState<'tcx>>,
    location: rustc_middle::mir::Location,
) -> Option<CombinedState<'tcx>> {
    state_before_location_from_result(tcx, body, result, location, transfer_stmt)
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
    let warn_on_maybe = load_analysis_bool_config("warn_on_maybe", false);

    let max_iterations = match (
        interval_config.max_iterations,
        nullptr_config.max_iterations,
    ) {
        (Some(left), Some(right)) => Some(left.min(right)),
        (Some(value), None) | (None, Some(value)) => Some(value),
        (None, None) => None,
    };
    let result = analyze_combined_with_config(
        tcx,
        body,
        cfg,
        places,
        all_places,
        pointer_places,
        PathForwardAnalysisConfig {
            max_paths: interval_config.max_paths.max(nullptr_config.max_paths),
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
    let debug = options.debug;
    let semantics = CombinedSemantics {
        places,
        all_places,
        pointer_places,
        interval_debug: debug,
        nullptr_debug: debug,
        symbolic_debug: debug,
        reduce_debug: debug,
    };
    run_path_sensitive_analysis(tcx, body, cfg, &semantics, analysis_config)
}
