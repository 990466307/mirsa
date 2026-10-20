use super::AnalysisSession;
use super::context::{InputContext, PlaceMap};
use crate::reduced_product::engine::{analyze_with_entry, invalidate_memory, transfer_stmt};
use crate::reduced_product::{AnalysisOptions, CombinedState};
use mirsa_core::cfg::build_cfg;
use mirsa_core::mir::{collect_body_places, collect_interval_places, collect_ptr_places};
use mirsa_domains::allocation::{AllocationState, abstract_value::AllocationId};
use mirsa_domains::interval::IntervalState;
use mirsa_domains::nullptr::NullPtrState;
use mirsa_framework::access_path::AccessPath;
use mirsa_framework::forward::DomainState;
use mirsa_framework::mapping::PathMapping;
use rustc_middle::mir::{
    Body, Local, LocalDecl, Place, Rvalue, SourceInfo, Statement, StatementKind, TerminatorKind,
};
use rustc_middle::ty::TyCtxt;
use std::collections::{HashMap, HashSet};

#[derive(Clone, Debug)]
pub(super) enum ExitEffects<'tcx> {
    Unreachable,
    Unknown,
    State(CombinedState<'tcx>),
}

#[derive(Clone, Debug)]
pub(super) struct FunctionSummary<'tcx> {
    pub input: InputContext<'tcx>,
    pub normal: ExitEffects<'tcx>,
    pub unwind: ExitEffects<'tcx>,
}

pub(super) fn project_state<'tcx>(
    state: &CombinedState<'tcx>,
    map: &impl PathMapping<'tcx>,
    objects: &impl Fn(&AllocationId) -> Option<AllocationId>,
) -> CombinedState<'tcx> {
    let mut out = CombinedState::from_parts(
        state.symbolic.project(map),
        state.interval.project(map),
        state.nullptr.project(map),
        state.allocation.project(map, objects),
        state.reduce_debug,
    );
    out.feasible = state.feasible;
    out.writes = state.writes.as_ref().map(|writes| map.paths(writes));
    out
}

pub(super) fn build<'tcx>(
    tcx: TyCtxt<'tcx>,
    session: &AnalysisSession<'tcx>,
    body: &Body<'tcx>,
    input: InputContext<'tcx>,
) -> FunctionSummary<'tcx> {
    let first_input = body.local_decls.len();
    let mut body = body.clone();
    for ty in &input.types {
        body.local_decls.push(LocalDecl::new(*ty, body.span));
    }
    let all_places = collect_body_places(tcx, &body);
    let places = collect_interval_places(tcx, &body);
    let pointer_places = collect_ptr_places(tcx, &body);
    let mut entry = CombinedState::new_with_debug(
        IntervalState::new_bot_state(
            tcx,
            &body.local_decls,
            &places,
            &all_places,
            body.arg_count,
            false,
        ),
        NullPtrState::new_bot_state(&pointer_places, body.arg_count, false),
        AllocationState::new_entry(tcx, &body, &all_places, false),
        false,
        false,
    );
    // Imported caller locals already have values. Missing facts mean unknown,
    // not the bottom used for callee temporaries before their first assignment.
    let inputs: Vec<_> = (first_input..body.local_decls.len())
        .map(|index| AccessPath::from_local(Local::from_usize(index)))
        .collect();
    invalidate_memory(&mut entry, &inputs);
    entry.extend(input.state.clone());
    for (index, arg) in input.args.iter().enumerate() {
        let Some(arg) = arg else {
            continue;
        };
        transfer_stmt(
            tcx,
            &mut entry,
            &Statement {
                source_info: SourceInfo::outermost(body.span),
                kind: StatementKind::Assign(Box::new((
                    Place::from(Local::from_usize(index + 1)),
                    Rvalue::Use(arg.clone()),
                ))),
            },
            &body.local_decls,
        );
    }
    for place in &all_places {
        let ty = place.ty(&body.local_decls, tcx).ty;
        let Some(pointee) = ty.builtin_deref(true) else {
            continue;
        };
        let pointer = entry
            .allocation
            .pointer_value_resolved(&entry.symbolic, *place);
        let Some((AllocationId::Stack(local), offset)) = pointer.exact_target() else {
            continue;
        };
        if offset.low == 0 && offset.high == 0 && pointee == body.local_decls[*local].ty {
            if let Some(path) = entry.symbolic.normalize_place(*place) {
                entry.symbolic.set_points_to(
                    path,
                    mirsa_framework::access_path::AccessPath::from_local(*local),
                );
            }
        }
    }
    entry.writes = Some(HashSet::new());
    let result = analyze_with_entry(
        tcx,
        &body,
        &build_cfg(&body),
        &places,
        &all_places,
        &pointer_places,
        session.remaining_config(),
        AnalysisOptions::default(),
        session,
        Some(&entry),
        None,
    );
    let normal = if !result.completed {
        ExitEffects::Unknown
    } else {
        let mut exit: Option<CombinedState<'tcx>> = None;
        for (bb, data) in body.basic_blocks.iter_enumerated() {
            if !result.reachable[bb.index()]
                || !matches!(data.terminator().kind, TerminatorKind::Return)
            {
                continue;
            }
            let mut state = result.out_states[bb.index()].clone();
            if !state.feasible {
                continue;
            }
            for place in &all_places {
                if place.local.index() == 0 || place.local.index() >= first_input {
                    state.reduce_place(tcx, &body.local_decls, *place);
                }
            }
            if !state.feasible {
                continue;
            }
            for index in 0..first_input {
                state
                    .allocation
                    .set_stack_live(Local::from_usize(index), false);
            }
            exit = Some(match exit {
                Some(previous) => CombinedState::join(&previous, &state),
                None => state,
            });
        }
        match exit {
            None => ExitEffects::Unreachable,
            Some(state) => {
                let locals: HashMap<_, _> = std::iter::once(0)
                    .chain(first_input..body.local_decls.len())
                    .map(|index| {
                        let local = Local::from_usize(index);
                        (local, Place::from(local))
                    })
                    .collect();
                let roots = locals.keys().copied().collect();
                let mut objects = state.allocation.referenced_objects(&roots);
                objects.extend(input.state.allocation.objects().map(|(id, _)| id.clone()));
                let map = PlaceMap {
                    tcx,
                    locals,
                    symbolic: Some(&state.symbolic),
                };
                ExitEffects::State(project_state(&state, &map, &|id| {
                    objects.contains(id).then(|| id.clone())
                }))
            }
        }
    };
    FunctionSummary {
        input,
        normal,
        unwind: ExitEffects::Unknown,
    }
}
