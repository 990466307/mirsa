use core::cfg::Cfg;
use rustc_hir::def_id::DefId;
use rustc_middle::mir::{Body, LocalDecls, Place, Statement};
use rustc_middle::ty::TyCtxt;
use std::path::Path;

use crate::framework::config::load_engine_config;
use crate::framework::forward::{ForwardSemantics, PathForwardAnalysisConfig};
use crate::framework::printer::run_and_print_path_sensitive_analysis;

use super::condition_path::refine_edge;
use super::state::SignState;
use super::transfer::transfer_stmt;

struct SignSemantics<'a, 'tcx> {
    places: &'a [Place<'tcx>],
}

impl<'a, 'tcx> ForwardSemantics<'tcx> for SignSemantics<'a, 'tcx> {
    type State = SignState<'tcx>;

    fn bottom(&self, body: &Body<'tcx>) -> Self::State {
        SignState::new_bot_state(self.places, body.arg_count)
    }

    fn transfer_stmt(
        &self,
        tcx: TyCtxt<'tcx>,
        st: &mut Self::State,
        stmt: &Statement<'tcx>,
        local_decls: &LocalDecls<'tcx>,
    ) {
        transfer_stmt(tcx, st, stmt, local_decls)
    }

    fn refine_edge(
        &self,
        tcx: TyCtxt<'tcx>,
        body: &Body<'tcx>,
        pred: rustc_middle::mir::BasicBlock,
        succ: rustc_middle::mir::BasicBlock,
        in_state: &Self::State,
    ) -> Option<Self::State> {
        refine_edge(tcx, body, pred, succ, in_state)
    }
}

pub fn run_sign<'tcx>(
    tcx: TyCtxt<'tcx>,
    def_id: DefId,
    body: &Body<'tcx>,
    cfg: &Cfg,
    places: &Vec<Place<'tcx>>,
) {
    let config = load_engine_config(Path::new("crates/domains/src/sign/sign.toml"));
    let semantics = SignSemantics { places };
    run_and_print_path_sensitive_analysis(
        tcx,
        def_id,
        body,
        cfg,
        &semantics,
        PathForwardAnalysisConfig {
            max_paths: config.max_paths,
            widen_after_iterations: config.max_iterations,
        },
    );
}
