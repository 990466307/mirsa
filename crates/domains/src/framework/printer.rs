use crate::framework::forward::{
    ForwardSemantics, PathForwardAnalysisConfig, PathForwardAnalysisResult,
    run_path_sensitive_forward_analysis_with_config,
};
use mirsa_core::cfg::Cfg;
use rustc_hir::def_id::DefId;
use rustc_middle::mir::{
    Body, Local, Location, Place, ProjectionElem, Terminator, TerminatorKind, VarDebugInfoContents,
};
use rustc_middle::ty::TyCtxt;
use std::collections::HashMap;

pub trait StateEntries<'tcx> {
    fn entries(&self) -> Vec<(Place<'tcx>, String)>;
    fn should_print_entry(&self, _place: Place<'tcx>) -> bool {
        true
    }
}

pub fn print_function_header<'tcx>(tcx: TyCtxt<'tcx>, def_id: DefId) {
    let name = tcx.def_path_str(def_id);
    println!("== Function: {name} ==");
}

pub fn pick_return_or_last_bb<'tcx>(body: &Body<'tcx>, states_len: usize) -> usize {
    let return_bb_idx = body
        .basic_blocks
        .iter_enumerated()
        .find(|(_, bbdata)| {
            bbdata.statements.is_empty()
                && matches!(
                    bbdata.terminator.as_ref().map(|t| &t.kind),
                    Some(TerminatorKind::Return)
                )
        })
        .map(|(bb, _)| bb.index());
    return_bb_idx.unwrap_or_else(|| states_len.saturating_sub(1))
}

pub fn collect_local_names<'tcx>(body: &Body<'tcx>) -> HashMap<Local, String> {
    let mut names = HashMap::new();
    for info in &body.var_debug_info {
        if let VarDebugInfoContents::Place(place) = &info.value {
            if place.projection.is_empty() {
                names
                    .entry(place.local)
                    .or_insert_with(|| info.name.to_string());
            }
        }
    }
    names
}

pub fn format_place_label<'tcx>(
    place: Place<'tcx>,
    local_names: &HashMap<Local, String>,
) -> String {
    let mut label = local_names
        .get(&place.local)
        .cloned()
        .unwrap_or_else(|| format!("{:?}", place.local));

    for elem in place.projection.iter() {
        match elem {
            ProjectionElem::ConstantIndex {
                offset,
                from_end: false,
                ..
            } => {
                label.push_str(&format!("[{offset}]"));
            }
            ProjectionElem::Index(local) => {
                let idx = local_names
                    .get(&local)
                    .cloned()
                    .unwrap_or_else(|| format!("{local:?}"));
                label.push_str(&format!("[{idx}]"));
            }
            ProjectionElem::Field(field, _) => {
                label.push_str(&format!(".{}", field.as_usize()));
            }
            _ => return format!("{place:?}"),
        }
    }

    label
}

pub fn visible_entries<'tcx, S>(body: &Body<'tcx>, state: &S) -> Vec<(String, String)>
where
    S: StateEntries<'tcx>,
{
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

pub fn print_call_pre_states<'tcx, S>(
    tcx: TyCtxt<'tcx>,
    body: &Body<'tcx>,
    result: &PathForwardAnalysisResult<S>,
    mut state_before: impl FnMut(
        TyCtxt<'tcx>,
        &Body<'tcx>,
        &PathForwardAnalysisResult<S>,
        Location,
    ) -> Option<S>,
    mut should_print_call: impl FnMut(TyCtxt<'tcx>, &Body<'tcx>, &Terminator<'tcx>) -> bool,
) where
    S: StateEntries<'tcx>,
{
    for (bb, bbdata) in body.basic_blocks.iter_enumerated() {
        let Some(term) = bbdata.terminator.as_ref() else {
            continue;
        };
        let TerminatorKind::Call { .. } = &term.kind else {
            continue;
        };
        if !should_print_call(tcx, body, term) {
            continue;
        }
        let location = Location {
            block: bb,
            statement_index: bbdata.statements.len(),
        };
        let Some(state) = state_before(tcx, body, result, location) else {
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

pub fn run_and_print_path_sensitive_analysis<'tcx, A>(
    tcx: TyCtxt<'tcx>,
    def_id: DefId,
    body: &Body<'tcx>,
    cfg: &Cfg,
    semantics: &A,
    config: PathForwardAnalysisConfig,
) where
    A: ForwardSemantics<'tcx>,
    A::State: StateEntries<'tcx>,
{
    let result = run_path_sensitive_analysis(tcx, body, cfg, semantics, config);
    print_function_header(tcx, def_id);
    let picked_bb_idx = pick_return_or_last_bb(body, result.out_states.len());
    if let Some(state) = result.out_states.get(picked_bb_idx) {
        println!("  bb{picked_bb_idx}:");
        println!("  locals: {:?}", body.var_debug_info);
        let entries = visible_entries(body, state);
        let place_width = entries
            .iter()
            .map(|(place, _)| place.len())
            .max()
            .unwrap_or(0);
        for (place, value) in entries {
            println!("    {place:place_width$} => {value}");
        }
    }
}

pub fn run_path_sensitive_analysis<'tcx, A>(
    tcx: TyCtxt<'tcx>,
    body: &Body<'tcx>,
    cfg: &Cfg,
    semantics: &A,
    config: PathForwardAnalysisConfig,
) -> PathForwardAnalysisResult<A::State>
where
    A: ForwardSemantics<'tcx>,
{
    run_path_sensitive_forward_analysis_with_config(tcx, body, cfg, semantics, config)
}

pub fn print_all_bb_states<'tcx, S>(body: &Body<'tcx>, states: &[S])
where
    S: StateEntries<'tcx>,
{
    println!("  locals: {:?}", body.var_debug_info);
    for (bb, state) in body
        .basic_blocks
        .iter_enumerated()
        .filter_map(|(bb, _)| states.get(bb.index()).map(|state| (bb, state)))
    {
        println!("  bb{}:", bb.index());
        let entries = visible_entries(body, state);
        let place_width = entries
            .iter()
            .map(|(place, _)| place.len())
            .max()
            .unwrap_or(0);
        for (place, value) in entries {
            println!("    {place:place_width$} => {value}");
        }
    }
}

pub fn print_final_analysis_result<'tcx, S>(
    body: &Body<'tcx>,
    result: &PathForwardAnalysisResult<S>,
) where
    S: StateEntries<'tcx>,
{
    println!("  final in-states:");
    print_all_bb_states(body, &result.in_states);

    println!("  final out-states:");
    print_all_bb_states(body, &result.out_states);
}
