use super::{branch, state::CombinedState};
use crate::contracts::combined::emit_combined_warnings;
use crate::framework::config::{load_bool_config, load_engine_config};
use crate::framework::forward::{
    ForwardSemantics, PathForwardAnalysisConfig, PathForwardAnalysisResult,
    state_before_location_from_result,
};
use crate::framework::printer::{
    print_call_pre_states, print_final_analysis_result, print_function_header,
    run_path_sensitive_analysis,
};
use crate::framework::symbolic as symbolic_transfer;
use crate::interval::IntervalState;
use crate::interval::transfer as interval_transfer;
use crate::nullptr::NullPtrState;
use crate::nullptr::transfer as nullptr_transfer;
use mirsa_core::cfg::Cfg;
use rustc_hir::def_id::DefId;
use rustc_middle::mir::{BasicBlock, Body, LocalDecls, Place, Statement, Terminator};
use rustc_middle::ty::TyCtxt;
use std::path::Path;

struct CombinedSemantics<'a, 'tcx> {
    places: &'a [Place<'tcx>],
    pointer_places: &'a [Place<'tcx>],
    interval_debug: bool,
    nullptr_debug: bool,
}

impl<'a, 'tcx> ForwardSemantics<'tcx> for CombinedSemantics<'a, 'tcx> {
    type State = CombinedState<'tcx>;

    fn bottom(&self, body: &Body<'tcx>) -> Self::State {
        CombinedState::new(
            IntervalState::new_bot_state(self.places, body.arg_count, self.interval_debug),
            NullPtrState::new_bot_state(self.pointer_places, body.arg_count, self.nullptr_debug),
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
        interval_transfer::transfer_stmt(tcx, &mut st.interval, stmt, local_decls);
        nullptr_transfer::transfer_stmt(tcx, &mut st.nullptr, stmt, local_decls);
        let _ = st.reduce_with_context(tcx, local_decls);
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
        nullptr_transfer::transfer_terminator(tcx, &mut st.nullptr, term, local_decls);
        let _ = st.reduce_with_context(tcx, local_decls);
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
    interval_transfer::transfer_stmt(tcx, &mut st.interval, stmt, local_decls);
    nullptr_transfer::transfer_stmt(tcx, &mut st.nullptr, stmt, local_decls);
    let _ = st.reduce_with_context(tcx, local_decls);
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
        |tcx, body, term| crate::contracts::matcher::classify_call(tcx, body, term).is_some(),
    );
}

pub fn run_combined<'tcx>(
    tcx: TyCtxt<'tcx>,
    def_id: DefId,
    body: &Body<'tcx>,
    cfg: &Cfg,
    places: &[Place<'tcx>],
    pointer_places: &[Place<'tcx>],
) {
    let interval_config_path = Path::new("crates/domains/src/interval/interval.toml");
    let nullptr_config_path = Path::new("crates/domains/src/nullptr/nullptr.toml");
    let interval_config = load_engine_config(interval_config_path);
    let nullptr_config = load_engine_config(nullptr_config_path);
    let interval_debug = load_bool_config(interval_config_path, "debug", false);
    let nullptr_debug = load_bool_config(nullptr_config_path, "debug", false);
    let warn_on_maybe = load_bool_config(nullptr_config_path, "warn_on_maybe", false);

    let max_iterations = match (
        interval_config.max_iterations,
        nullptr_config.max_iterations,
    ) {
        (Some(left), Some(right)) => Some(left.min(right)),
        (Some(value), None) | (None, Some(value)) => Some(value),
        (None, None) => None,
    };
    let semantics = CombinedSemantics {
        places,
        pointer_places,
        interval_debug,
        nullptr_debug,
    };
    let result = run_path_sensitive_analysis(
        tcx,
        body,
        cfg,
        &semantics,
        PathForwardAnalysisConfig {
            max_paths: interval_config.max_paths.max(nullptr_config.max_paths),
            widen_after_iterations: max_iterations,
        },
    );

    print_function_header(tcx, def_id);
    if interval_debug || nullptr_debug {
        print_final_analysis_result(body, &result);
    }
    print_unsafe_pre_states(tcx, body, &result);

    emit_combined_warnings(tcx, body, &result, warn_on_maybe);
}
