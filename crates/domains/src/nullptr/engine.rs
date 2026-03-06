use core::cfg::Cfg;
use rustc_hir::def_id::DefId;
use rustc_middle::mir::{BasicBlock, Body, LocalDecls, Place, Statement};
use rustc_middle::ty::TyCtxt;
use std::path::Path;

use crate::framework::config::load_engine_config;
use crate::framework::forward::{ForwardSemantics, PathForwardAnalysisConfig};
use crate::framework::printer::run_and_print_path_sensitive_analysis;

use super::condition_path::refine_edge;
use super::state::NullPtrState;
use super::transfer::{transfer_stmt, transfer_terminator};

struct NullPtrSemantics<'a, 'tcx> {
    places: &'a [Place<'tcx>],
    refs: &'a [Place<'tcx>],
}

impl<'a, 'tcx> ForwardSemantics<'tcx> for NullPtrSemantics<'a, 'tcx> {
    type State = NullPtrState<'tcx>;

    fn bottom(&self, body: &'tcx Body<'tcx>) -> Self::State {
        NullPtrState::new_bot_state(self.places, self.refs, body.arg_count)
    }

    fn transfer_stmt(
        &self,
        tcx: TyCtxt<'tcx>,
        st: &mut Self::State,
        stmt: &Statement<'tcx>,
        local_decls: &'tcx LocalDecls<'tcx>,
    ) {
        transfer_stmt(tcx, st, stmt, local_decls)
    }

    fn transfer_terminator(
        &self,
        tcx: TyCtxt<'tcx>,
        st: &mut Self::State,
        term: &rustc_middle::mir::Terminator<'tcx>,
        local_decls: &'tcx LocalDecls<'tcx>,
    ) {
        transfer_terminator(tcx, st, term, local_decls)
    }

    fn refine_edge(
        &self,
        tcx: TyCtxt<'tcx>,
        body: &'tcx Body<'tcx>,
        pred: BasicBlock,
        succ: BasicBlock,
        in_state: &Self::State,
    ) -> Option<Self::State> {
        refine_edge(tcx, body, pred, succ, in_state)
    }
}

pub fn run_nullptr<'tcx>(
    tcx: TyCtxt<'tcx>,
    def_id: DefId,
    body: &'tcx Body<'tcx>,
    cfg: &Cfg,
    places: &Vec<Place<'tcx>>,
    ref_places: &Vec<Place<'tcx>>,
) {
    let config = load_engine_config(Path::new("crates/domains/src/nullptr/nullptr.toml"));
    let semantics = NullPtrSemantics {
        places,
        refs: ref_places,
    };

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
