use super::condition_path::refine_edge;
use super::state::InternvalState;
use super::transfer::{transfer_stmt, transfer_terminator};
use super::warnings::{emit_internval_warnings, is_supported_unsafe_call};
use crate::framework::config::load_engine_config;
use crate::framework::forward::{
    ForwardSemantics, PathForwardAnalysisConfig, PathForwardAnalysisResult,
    state_before_location_from_result,
};
use crate::framework::printer::{
    StateEntries, collect_local_names, format_place_label, print_function_header,
    run_path_sensitive_analysis,
};
use core::cfg::Cfg;
use rustc_hir::def_id::DefId;
use rustc_middle::mir::{BasicBlock, Body, LocalDecls, Place, Statement, Terminator, TerminatorKind};
use rustc_middle::ty::TyCtxt;
use std::path::Path;

struct InternvalSemantics<'a, 'tcx> {
    places: &'a [Place<'tcx>],
}

impl<'a, 'tcx> ForwardSemantics<'tcx> for InternvalSemantics<'a, 'tcx> {
    type State = InternvalState<'tcx>;

    fn bottom(&self, body: &Body<'tcx>) -> Self::State {
        InternvalState::new_bot_state(self.places, body.arg_count)
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

    fn transfer_terminator(
        &self,
        tcx: TyCtxt<'tcx>,
        st: &mut Self::State,
        term: &Terminator<'tcx>,
        local_decls: &LocalDecls<'tcx>,
    ) {
        transfer_terminator(tcx, st, term, local_decls)
    }

    fn refine_edge(
        &self,
        tcx: TyCtxt<'tcx>,
        body: &Body<'tcx>,
        pred: BasicBlock,
        succ: BasicBlock,
        in_state: &Self::State,
    ) -> Option<Self::State> {
        refine_edge(tcx, body, pred, succ, in_state)
    }
}

fn visible_entries<'tcx>(
    body: &Body<'tcx>,
    state: &InternvalState<'tcx>,
) -> Vec<(String, String)> {
    let local_names = collect_local_names(body);
    let mut entries: Vec<(String, String)> = state
        .entries()
        .into_iter()
        .filter(|(place, _)| state.should_print_entry(*place))
        .map(|(place, value)| (format_place_label(place, &local_names), value))
        .filter(|(label, _)| !label.starts_with('_'))
        .collect();
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    entries.dedup();
    entries
}

fn print_unsafe_pre_states<'tcx>(
    tcx: TyCtxt<'tcx>,
    body: &Body<'tcx>,
    result: &PathForwardAnalysisResult<InternvalState<'tcx>>,
) {
    for (bb, bbdata) in body.basic_blocks.iter_enumerated() {
        let Some(term) = bbdata.terminator.as_ref() else {
            continue;
        };
        let TerminatorKind::Call { .. } = &term.kind else {
            continue;
        };
        if !is_supported_unsafe_call(tcx, body, term) {
            continue;
        }
        let location = rustc_middle::mir::Location {
            block: bb,
            statement_index: bbdata.statements.len(),
        };
        let Some(state) = state_before_location(tcx, body, result, location) else {
            continue;
        };
        let entries = visible_entries(body, &state);
        if entries.is_empty() {
            continue;
        }
        println!("  unsafe pre-state @ bb{}:", bb.index());
        let width = entries
            .iter()
            .map(|(label, _)| label.len())
            .max()
            .unwrap_or(0);
        for (label, value) in entries {
            println!("    {label:width$} => {value}");
        }
    }
}

fn has_supported_unsafe_calls<'tcx>(tcx: TyCtxt<'tcx>, body: &Body<'tcx>) -> bool {
    body.basic_blocks.iter().any(|bbdata| {
        bbdata
            .terminator
            .as_ref()
            .is_some_and(|term| is_supported_unsafe_call(tcx, body, term))
    })
}

pub fn analyze_internval<'tcx>(
    tcx: TyCtxt<'tcx>,
    body: &Body<'tcx>,
    cfg: &Cfg,
    places: &[Place<'tcx>],
    config: PathForwardAnalysisConfig,
) -> PathForwardAnalysisResult<InternvalState<'tcx>> {
    let semantics = InternvalSemantics { places };
    run_path_sensitive_analysis(tcx, body, cfg, &semantics, config)
}

pub fn state_before_location<'tcx>(
    tcx: TyCtxt<'tcx>,
    body: &Body<'tcx>,
    result: &PathForwardAnalysisResult<InternvalState<'tcx>>,
    location: rustc_middle::mir::Location,
) -> Option<InternvalState<'tcx>> {
    state_before_location_from_result(tcx, body, result, location, transfer_stmt)
}

pub fn run_internval<'tcx>(
    tcx: TyCtxt<'tcx>,
    def_id: DefId,
    body: &Body<'tcx>,
    cfg: &Cfg,
    places: &Vec<Place<'tcx>>,
) {
    if !has_supported_unsafe_calls(tcx, body) {
        return;
    }
    let config = load_engine_config(Path::new("crates/domains/src/internval/internval.toml"));
    let result = analyze_internval(
        tcx,
        body,
        cfg,
        places,
        PathForwardAnalysisConfig {
            max_paths: config.max_paths,
            widen_after_iterations: config.max_iterations,
        },
    );
    print_function_header(tcx, def_id);
    print_unsafe_pre_states(tcx, body, &result);
    emit_internval_warnings(tcx, body, &result);
}
