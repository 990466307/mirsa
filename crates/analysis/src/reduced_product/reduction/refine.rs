use mirsa_domains::interval::abstract_value::intersect;
use mirsa_domains::interval::float_interval::intersect as intersect_float;
use mirsa_domains::interval::{FloatInterval, Interval, IntervalState};
use mirsa_domains::nullptr::constraint::constrain;
use mirsa_domains::nullptr::transfer::{get_tracked_value, is_tracked};
use mirsa_domains::nullptr::{NullPtr, NullPtrState};
use mirsa_framework::access_path::AccessPath;
use mirsa_framework::reduction::MAX_RELATED;
use mirsa_relations::symbolic::SymbolicState;
use rustc_middle::mir::Place;
use rustc_middle::ty::Ty;

pub(super) fn refine_integer_place<'tcx>(
    state: &mut IntervalState<'tcx>,
    symbolic: &SymbolicState<'tcx>,
    place: Place<'tcx>,
    wanted: Interval,
) -> bool {
    let current = state.read_interval_resolved(symbolic, place);
    if current.is_empty() {
        return true;
    }
    let refined = intersect(&current, &wanted);
    if refined.is_empty() {
        return false;
    }
    state.debug(format_args!(
        "reduce {:?}: {} ∩ {} = {}",
        place, current, wanted, refined
    ));
    state.set_interval_resolved(symbolic, place, refined);
    for other in symbolic.equivalent_places(place, MAX_RELATED) {
        if other == place || !symbolic.equiv_places_readonly(place, other) {
            continue;
        }
        let other_current = state.read_interval_resolved(symbolic, other);
        if other_current.is_empty() {
            continue;
        }
        let other_refined = intersect(&other_current, &refined);
        if other_refined.is_empty() {
            return false;
        }
        state.set_interval_resolved(symbolic, other, other_refined);
    }
    true
}

pub(super) fn refine_float_place<'tcx>(
    state: &mut IntervalState<'tcx>,
    symbolic: &SymbolicState<'tcx>,
    place: Place<'tcx>,
    wanted: FloatInterval,
) -> bool {
    let current = state.read_float_interval_resolved(symbolic, place);
    if current.is_bottom() {
        return true;
    }
    let refined = intersect_float(&current, &wanted);
    if refined.is_bottom() {
        return false;
    }
    state.debug(format_args!(
        "reduce float {:?}: {} ∩ {} = {}",
        place, current, wanted, refined
    ));
    state.set_float_interval_resolved(symbolic, place, refined);
    for other in symbolic.equivalent_places(place, MAX_RELATED) {
        if other == place || !symbolic.equiv_places_readonly(place, other) {
            continue;
        }
        let other_current = state.read_float_interval_resolved(symbolic, other);
        if other_current.is_bottom() {
            continue;
        }
        let other_refined = intersect_float(&other_current, &refined);
        if other_refined.is_bottom() {
            return false;
        }
        state.set_float_interval_resolved(symbolic, other, other_refined);
    }
    true
}

pub(super) fn refine_nullptr_place<'tcx>(
    state: &mut NullPtrState<'tcx>,
    symbolic: &SymbolicState<'tcx>,
    place: Place<'tcx>,
    ty: Ty<'tcx>,
    wanted: NullPtr,
) -> bool {
    if !is_tracked(ty) {
        return true;
    }
    let current = get_tracked_value(state, symbolic, place, ty);
    if current == NullPtr::Bot {
        return true;
    }
    let Some(refined) = constrain(current, wanted) else {
        return false;
    };
    state.debug(format_args!(
        "reduce {:?}: {:?} ∩ {:?} = {:?}",
        place, current, wanted, refined
    ));
    let Some(path) = state.access_path_for_place_resolved(symbolic, place) else {
        return true;
    };
    refine_equivalent_facts(state, symbolic, path, refined)
}

fn refine_equivalent_facts<'tcx>(
    state: &mut NullPtrState<'tcx>,
    symbolic: &SymbolicState<'tcx>,
    path: AccessPath,
    refined: NullPtr,
) -> bool {
    state.set_path(path.clone(), refined);
    let paths: Vec<_> = symbolic
        .eq
        .members(path.clone())
        .take(MAX_RELATED)
        .collect();
    for other in paths {
        if other == path || !symbolic.eq.equiv_readonly(path.clone(), other.clone()) {
            continue;
        }
        let current = state.value_or_maybe(&other);
        if current == NullPtr::Bot {
            continue;
        }
        let Some(value) = constrain(current, refined) else {
            return false;
        };
        state.set_path(other, value);
    }
    true
}
