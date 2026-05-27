use mirsa_core::cfg::Cfg;
use rustc_hir::def_id::DefId;
use rustc_middle::mir::{BasicBlock, Body, LocalDecls, Place, Statement};
use rustc_middle::ty::TyCtxt;
use std::path::Path;

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

use super::state::{NullPtrAnalysisState, NullPtrState};
use super::transfer::{transfer_stmt, transfer_terminator};
use crate::combined::{CombinedState, branch as combined_branch};
use crate::contracts::nullptr::{emit_nonnull_call_warnings, is_supported_unsafe_call};
use crate::interval::IntervalState;

struct NullPtrSemantics<'a, 'tcx> {
    places: &'a [Place<'tcx>],
    debug: bool,
}

impl<'a, 'tcx> ForwardSemantics<'tcx> for NullPtrSemantics<'a, 'tcx> {
    type State = NullPtrAnalysisState<'tcx>;

    fn bottom(&self, body: &Body<'tcx>) -> Self::State {
        NullPtrAnalysisState::new(NullPtrState::new_bot_state(
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
        transfer_stmt(tcx, &mut st.nullptr, stmt, local_decls)
    }

    fn transfer_terminator(
        &self,
        tcx: TyCtxt<'tcx>,
        st: &mut Self::State,
        term: &rustc_middle::mir::Terminator<'tcx>,
        local_decls: &LocalDecls<'tcx>,
    ) {
        symbolic_transfer::transfer_terminator(tcx, &mut st.symbolic, term, local_decls);
        transfer_terminator(tcx, &mut st.nullptr, term, local_decls)
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
            interval: IntervalState::new_bot_state(&[], body.arg_count, false),
            nullptr: in_state.nullptr.clone(),
        };
        let combined_out = combined_branch::refine_edge(tcx, body, pred, succ, &combined_in)?;
        Some(NullPtrAnalysisState {
            symbolic: combined_out.symbolic,
            nullptr: combined_out.nullptr,
        })
    }
}

fn print_unsafe_pre_states<'tcx>(
    tcx: TyCtxt<'tcx>,
    body: &Body<'tcx>,
    result: &PathForwardAnalysisResult<NullPtrAnalysisState<'tcx>>,
) {
    print_call_pre_states(
        tcx,
        body,
        result,
        state_before_location,
        is_supported_unsafe_call,
    );
}

pub fn analyze_nullptr<'tcx>(
    tcx: TyCtxt<'tcx>,
    body: &Body<'tcx>,
    cfg: &Cfg,
    places: &[Place<'tcx>],
    debug: bool,
    config: PathForwardAnalysisConfig,
) -> PathForwardAnalysisResult<NullPtrAnalysisState<'tcx>> {
    let semantics = NullPtrSemantics { places, debug };
    run_path_sensitive_analysis(tcx, body, cfg, &semantics, config)
}

pub fn state_before_location<'tcx>(
    tcx: TyCtxt<'tcx>,
    body: &Body<'tcx>,
    result: &PathForwardAnalysisResult<NullPtrAnalysisState<'tcx>>,
    location: rustc_middle::mir::Location,
) -> Option<NullPtrAnalysisState<'tcx>> {
    state_before_location_from_result(tcx, body, result, location, |tcx, st, stmt, local_decls| {
        symbolic_transfer::transfer_stmt(tcx, &mut st.symbolic, stmt, local_decls);
        transfer_stmt(tcx, &mut st.nullptr, stmt, local_decls)
    })
}

pub fn run_nullptr<'tcx>(
    tcx: TyCtxt<'tcx>,
    def_id: DefId,
    body: &Body<'tcx>,
    cfg: &Cfg,
    places: &Vec<Place<'tcx>>,
    _ref_places: &Vec<Place<'tcx>>,
) {
    let config_path = Path::new("crates/domains/src/nullptr/nullptr.toml");
    let config = load_engine_config(config_path);
    let debug = load_bool_config(config_path, "debug", false);
    let warn_on_maybe = load_bool_config(config_path, "warn_on_maybe", false);
    let result = analyze_nullptr(
        tcx,
        body,
        cfg,
        places,
        debug,
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
    emit_nonnull_call_warnings(tcx, body, &result, warn_on_maybe);
}
