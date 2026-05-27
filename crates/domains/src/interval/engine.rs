use super::state::{IntervalAnalysisState, IntervalState};
use super::transfer::{transfer_stmt, transfer_terminator};
use crate::combined::{CombinedState, branch as combined_branch};
use crate::contracts::interval::{emit_interval_warnings, is_supported_unsafe_call};
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
use crate::nullptr::NullPtrState;
use mirsa_core::cfg::Cfg;
use rustc_hir::def_id::DefId;
use rustc_middle::mir::{BasicBlock, Body, LocalDecls, Place, Statement, Terminator};
use rustc_middle::ty::TyCtxt;
use std::path::Path;

struct IntervalSemantics<'a, 'tcx> {
    places: &'a [Place<'tcx>],
    debug: bool,
}

impl<'a, 'tcx> ForwardSemantics<'tcx> for IntervalSemantics<'a, 'tcx> {
    type State = IntervalAnalysisState<'tcx>;

    fn bottom(&self, body: &Body<'tcx>) -> Self::State {
        IntervalAnalysisState::new(IntervalState::new_bot_state(
            self.places,
            body.arg_count,
            self.debug,
        ))
    }

    fn transfer_stmt(
        &self,
        tcx: TyCtxt<'tcx>,
        st: &mut Self::State,
        stmt: &Statement<'tcx>,
        local_decls: &LocalDecls<'tcx>,
    ) {
        symbolic_transfer::transfer_stmt(tcx, &mut st.symbolic, stmt, local_decls);
        transfer_stmt(tcx, &mut st.interval, stmt, local_decls)
    }

    fn transfer_terminator(
        &self,
        tcx: TyCtxt<'tcx>,
        st: &mut Self::State,
        term: &Terminator<'tcx>,
        local_decls: &LocalDecls<'tcx>,
    ) {
        symbolic_transfer::transfer_terminator(tcx, &mut st.symbolic, term, local_decls);
        transfer_terminator(tcx, &st.symbolic, &mut st.interval, term, local_decls)
    }

    fn refine_edge(
        &self,
        tcx: TyCtxt<'tcx>,
        body: &Body<'tcx>,
        pred: BasicBlock,
        succ: BasicBlock,
        in_state: &Self::State,
    ) -> Option<Self::State> {
        let combined_in = CombinedState {
            symbolic: in_state.symbolic.clone(),
            interval: in_state.interval.clone(),
            nullptr: NullPtrState::new_bot_state(&[], body.arg_count, false),
        };
        let combined_out = combined_branch::refine_edge(tcx, body, pred, succ, &combined_in)?;
        Some(IntervalAnalysisState {
            symbolic: combined_out.symbolic,
            interval: combined_out.interval,
        })
    }
}

fn print_unsafe_pre_states<'tcx>(
    tcx: TyCtxt<'tcx>,
    body: &Body<'tcx>,
    result: &PathForwardAnalysisResult<IntervalAnalysisState<'tcx>>,
) {
    print_call_pre_states(
        tcx,
        body,
        result,
        state_before_location,
        is_supported_unsafe_call,
    );
}

pub fn analyze_interval<'tcx>(
    tcx: TyCtxt<'tcx>,
    body: &Body<'tcx>,
    cfg: &Cfg,
    places: &[Place<'tcx>],
    config: PathForwardAnalysisConfig,
) -> PathForwardAnalysisResult<IntervalAnalysisState<'tcx>> {
    let semantics = IntervalSemantics {
        places,
        debug: false,
    };
    run_path_sensitive_analysis(tcx, body, cfg, &semantics, config)
}

pub fn state_before_location<'tcx>(
    tcx: TyCtxt<'tcx>,
    body: &Body<'tcx>,
    result: &PathForwardAnalysisResult<IntervalAnalysisState<'tcx>>,
    location: rustc_middle::mir::Location,
) -> Option<IntervalAnalysisState<'tcx>> {
    state_before_location_from_result(tcx, body, result, location, |tcx, st, stmt, local_decls| {
        symbolic_transfer::transfer_stmt(tcx, &mut st.symbolic, stmt, local_decls);
        transfer_stmt(tcx, &mut st.interval, stmt, local_decls)
    })
}

pub fn run_interval<'tcx>(
    tcx: TyCtxt<'tcx>,
    def_id: DefId,
    body: &Body<'tcx>,
    cfg: &Cfg,
    places: &Vec<Place<'tcx>>,
) {
    let config_path = Path::new("crates/domains/src/interval/interval.toml");
    let config = load_engine_config(config_path);
    let debug = load_bool_config(config_path, "debug", false);
    let semantics = IntervalSemantics { places, debug };
    let result = run_path_sensitive_analysis(
        tcx,
        body,
        cfg,
        &semantics,
        PathForwardAnalysisConfig {
            max_paths: config.max_paths,
            widen_after_iterations: config.max_iterations,
        },
    );
    print_function_header(tcx, def_id);
    if debug {
        print_final_analysis_result(body, &result);
    }
    print_unsafe_pre_states(tcx, body, &result);
    emit_interval_warnings(tcx, body, &result);
}
