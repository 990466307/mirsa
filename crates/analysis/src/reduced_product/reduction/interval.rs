use mirsa_domains::interval::abstract_value::join;
use mirsa_domains::interval::float_interval::join as join_float;
use mirsa_domains::interval::state::IntervalState;
use mirsa_domains::interval::transfer::{
    eval_assign_rhs_float, eval_assign_rhs_interval, float_kind_for_ty,
};
use mirsa_domains::interval::{FloatInterval, Interval};
use mirsa_relations::symbolic::SymbolicState;
use rustc_middle::mir::{
    LocalDecls, Operand, Place, ProjectionElem, Rvalue, Statement, StatementKind,
};
use rustc_middle::ty::{Ty, TyCtxt, TyKind};

const MAX_PRECOLLECT_ARRAY_ELEMENTS: u64 = 32;

#[derive(Clone, Debug)]
pub(super) enum ResolvedPlaces<'tcx> {
    Exact(Place<'tcx>),
    Candidates(Vec<Place<'tcx>>),
    Summary(Place<'tcx>),
}

pub fn reduce_statement<'tcx>(
    tcx: TyCtxt<'tcx>,
    state: &mut IntervalState<'tcx>,
    symbolic: &SymbolicState<'tcx>,
    statement: &Statement<'tcx>,
    local_decls: &LocalDecls<'tcx>,
) {
    let StatementKind::Assign(assign) = &statement.kind else {
        return;
    };
    let (destination, rvalue) = &**assign;

    if has_runtime_index(*destination) {
        let targets = resolve_places(tcx, local_decls, state, symbolic, *destination);
        reduce_write(tcx, local_decls, state, symbolic, &targets, rvalue);
    }

    let Rvalue::Use(Operand::Copy(source) | Operand::Move(source)) = rvalue else {
        return;
    };
    if has_runtime_index(*source) && !has_runtime_index(*destination) {
        let targets = resolve_places(tcx, local_decls, state, symbolic, *source);
        reduce_read(tcx, local_decls, state, symbolic, *destination, &targets);
    }
}

pub(super) fn has_runtime_index(place: Place<'_>) -> bool {
    place
        .projection
        .iter()
        .any(|elem| matches!(elem, ProjectionElem::Index(_)))
}

pub(super) fn resolve_places<'tcx>(
    tcx: TyCtxt<'tcx>,
    local_decls: &LocalDecls<'tcx>,
    state: &mut IntervalState<'tcx>,
    symbolic: &SymbolicState<'tcx>,
    place: Place<'tcx>,
) -> ResolvedPlaces<'tcx> {
    if let Some(place) = resolve_exact(tcx, local_decls, state, symbolic, place) {
        ResolvedPlaces::Exact(place)
    } else if let Some(places) = resolve_candidates(tcx, local_decls, state, symbolic, place) {
        ResolvedPlaces::Candidates(places)
    } else {
        ResolvedPlaces::Summary(place)
    }
}

pub(super) fn first_resolved_place<'tcx>(places: &ResolvedPlaces<'tcx>) -> Option<Place<'tcx>> {
    match places {
        ResolvedPlaces::Exact(place) | ResolvedPlaces::Summary(place) => Some(*place),
        ResolvedPlaces::Candidates(places) => places.first().copied(),
    }
}

fn reduce_read<'tcx>(
    tcx: TyCtxt<'tcx>,
    local_decls: &LocalDecls<'tcx>,
    state: &mut IntervalState<'tcx>,
    symbolic: &SymbolicState<'tcx>,
    destination: Place<'tcx>,
    places: &ResolvedPlaces<'tcx>,
) {
    let ty = destination.ty(local_decls, tcx).ty;
    if is_integer_scalar(ty) {
        let value = read_integer(state, symbolic, places);
        state.debug(format_args!(
            "reduce indexed read {:?} := {}",
            destination, value
        ));
        state.set_interval_resolved(symbolic, destination, value);
    } else if float_kind_for_ty(ty).is_some() {
        let value = read_float(state, symbolic, places);
        state.debug(format_args!(
            "reduce indexed float read {:?} := {}",
            destination, value
        ));
        state.set_float_interval_resolved(symbolic, destination, value);
    }
}

fn reduce_write<'tcx>(
    tcx: TyCtxt<'tcx>,
    local_decls: &LocalDecls<'tcx>,
    state: &mut IntervalState<'tcx>,
    symbolic: &SymbolicState<'tcx>,
    places: &ResolvedPlaces<'tcx>,
    rvalue: &Rvalue<'tcx>,
) {
    let Some(place) = first_resolved_place(places) else {
        return;
    };
    let ty = place.ty(local_decls, tcx).ty;
    if is_integer_scalar(ty) {
        let value = eval_assign_rhs_interval(tcx, state, symbolic, local_decls, rvalue);
        write_integer(state, symbolic, places, value);
    } else if float_kind_for_ty(ty).is_some() {
        let value = eval_assign_rhs_float(tcx, state, symbolic, local_decls, rvalue);
        write_float(state, symbolic, places, value);
    }
}

fn resolve_exact<'tcx>(
    tcx: TyCtxt<'tcx>,
    local_decls: &LocalDecls<'tcx>,
    state: &mut IntervalState<'tcx>,
    symbolic: &SymbolicState<'tcx>,
    place: Place<'tcx>,
) -> Option<Place<'tcx>> {
    let mut resolved = Place::from(place.local);
    for elem in place.projection.iter() {
        match elem {
            ProjectionElem::Index(local) => {
                let index = state.read_interval_resolved(symbolic, Place::from(local));
                if index.is_empty() || index.low != index.high || index.low < 0 {
                    return None;
                }
                let len = array_len(tcx, local_decls, resolved)?;
                let index = index.low as u64;
                if index >= len {
                    return None;
                }
                resolved = resolved.project_deeper(
                    &[ProjectionElem::ConstantIndex {
                        offset: index,
                        min_length: len,
                        from_end: false,
                    }],
                    tcx,
                );
            }
            _ => resolved = resolved.project_deeper(&[elem], tcx),
        }
    }
    Some(resolved)
}

fn resolve_candidates<'tcx>(
    tcx: TyCtxt<'tcx>,
    local_decls: &LocalDecls<'tcx>,
    state: &mut IntervalState<'tcx>,
    symbolic: &SymbolicState<'tcx>,
    place: Place<'tcx>,
) -> Option<Vec<Place<'tcx>>> {
    let mut candidates = vec![Place::from(place.local)];
    for elem in place.projection.iter() {
        match elem {
            ProjectionElem::Index(local) => {
                let index = state.read_interval_resolved(symbolic, Place::from(local));
                let mut next = Vec::new();
                for base in candidates {
                    let len = array_len(tcx, local_decls, base)?;
                    if index.is_empty() || len == 0 {
                        continue;
                    }
                    let low = index.low.max(0);
                    let high = index.high.min(len as i128 - 1);
                    for index in low..=high {
                        next.push(base.project_deeper(
                            &[ProjectionElem::ConstantIndex {
                                offset: index as u64,
                                min_length: len,
                                from_end: false,
                            }],
                            tcx,
                        ));
                    }
                }
                candidates = next;
            }
            _ => {
                candidates = candidates
                    .into_iter()
                    .map(|base| base.project_deeper(&[elem], tcx))
                    .collect();
            }
        }
    }
    Some(candidates)
}

fn array_len<'tcx>(
    tcx: TyCtxt<'tcx>,
    local_decls: &LocalDecls<'tcx>,
    place: Place<'tcx>,
) -> Option<u64> {
    let TyKind::Array(_, len) = place.ty(local_decls, tcx).ty.kind() else {
        return None;
    };
    let len = len.try_to_target_usize(tcx)? as u64;
    (len <= MAX_PRECOLLECT_ARRAY_ELEMENTS).then_some(len)
}

fn read_integer<'tcx>(
    state: &mut IntervalState<'tcx>,
    symbolic: &SymbolicState<'tcx>,
    places: &ResolvedPlaces<'tcx>,
) -> Interval {
    match places {
        ResolvedPlaces::Exact(place) => state.read_interval_resolved(symbolic, *place),
        ResolvedPlaces::Candidates(places) => places
            .iter()
            .copied()
            .map(|place| state.read_interval_resolved(symbolic, place))
            .reduce(|left, right| join(&left, &right))
            .unwrap_or_else(Interval::top),
        ResolvedPlaces::Summary(place) => state
            .tracked_interval_resolved(symbolic, place)
            .unwrap_or_else(Interval::top),
    }
}

fn read_float<'tcx>(
    state: &mut IntervalState<'tcx>,
    symbolic: &SymbolicState<'tcx>,
    places: &ResolvedPlaces<'tcx>,
) -> FloatInterval {
    match places {
        ResolvedPlaces::Exact(place) => state.read_float_interval_resolved(symbolic, *place),
        ResolvedPlaces::Candidates(places) => places
            .iter()
            .copied()
            .map(|place| state.read_float_interval_resolved(symbolic, place))
            .reduce(|left, right| join_float(&left, &right))
            .unwrap_or_else(FloatInterval::top),
        ResolvedPlaces::Summary(place) => state
            .tracked_float_interval_resolved(symbolic, place)
            .unwrap_or_else(FloatInterval::top),
    }
}

fn write_integer<'tcx>(
    state: &mut IntervalState<'tcx>,
    symbolic: &SymbolicState<'tcx>,
    places: &ResolvedPlaces<'tcx>,
    value: Interval,
) {
    match places {
        ResolvedPlaces::Exact(place) => {
            state.debug(format_args!(
                "reduce indexed write {:?} := {}",
                place, value
            ));
            state.set_interval_resolved(symbolic, *place, value);
        }
        ResolvedPlaces::Candidates(places) => {
            for place in places {
                state.debug(format_args!(
                    "reduce indexed weak write {:?} := {}",
                    place, value
                ));
                state.join_interval_resolved(symbolic, *place, value);
            }
        }
        ResolvedPlaces::Summary(place) => {
            state.debug(format_args!(
                "reduce indexed summary write {:?} := {}",
                place, value
            ));
            state.join_interval_resolved(symbolic, *place, value);
        }
    }
}

fn write_float<'tcx>(
    state: &mut IntervalState<'tcx>,
    symbolic: &SymbolicState<'tcx>,
    places: &ResolvedPlaces<'tcx>,
    value: FloatInterval,
) {
    match places {
        ResolvedPlaces::Exact(place) => {
            state.debug(format_args!(
                "reduce indexed float write {:?} := {}",
                place, value
            ));
            state.set_float_interval_resolved(symbolic, *place, value);
        }
        ResolvedPlaces::Candidates(places) => {
            for place in places {
                state.debug(format_args!(
                    "reduce indexed float weak write {:?} := {}",
                    place, value
                ));
                state.join_float_interval_resolved(symbolic, *place, value);
            }
        }
        ResolvedPlaces::Summary(place) => {
            state.debug(format_args!(
                "reduce indexed float summary write {:?} := {}",
                place, value
            ));
            state.join_float_interval_resolved(symbolic, *place, value);
        }
    }
}

fn is_integer_scalar(ty: Ty<'_>) -> bool {
    matches!(
        ty.kind(),
        TyKind::Int(_) | TyKind::Uint(_) | TyKind::Bool | TyKind::Char
    )
}
