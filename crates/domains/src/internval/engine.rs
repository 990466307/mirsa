use super::condition_path::refine_edge;
use super::state::InternvalState;
use super::transfer::transfer_stmt;
use crate::framework::config::load_engine_config;
use crate::framework::forward::{ForwardSemantics, PathForwardAnalysisConfig};
use crate::framework::printer::run_and_print_path_sensitive_analysis;
use core::cfg::Cfg;
use rustc_hir::def_id::DefId;
use rustc_middle::mir::{BasicBlock, Body, LocalDecls, Place, Statement};
use rustc_middle::ty::TyCtxt;
use std::path::Path;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PathState<'tcx> {
    pub states: Vec<InternvalState<'tcx>>,
    pub iterations: Vec<u32>,
    pub visited: Vec<bool>,
    pub is_abstract: bool, // true表示是多条路径的抽象
}

struct InternvalSemantics<'a, 'tcx> {
    places: &'a [Place<'tcx>],
}

impl<'a, 'tcx> ForwardSemantics<'tcx> for InternvalSemantics<'a, 'tcx> {
    type State = InternvalState<'tcx>;

    fn bottom(&self, body: &'tcx Body<'tcx>) -> Self::State {
        InternvalState::new_bot_state(self.places, body.arg_count)
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

pub fn run_internval<'tcx>(
    tcx: TyCtxt<'tcx>,
    def_id: DefId,
    body: &'tcx Body<'tcx>,
    cfg: &Cfg,
    places: &Vec<Place<'tcx>>,
) {
    let config = load_engine_config(Path::new("crates/domains/src/internval/internval.toml"));
    let semantics = InternvalSemantics { places };
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
