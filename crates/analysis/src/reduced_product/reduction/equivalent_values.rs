use super::refine;
use crate::reduced_product::state::CombinedState;
use mirsa_domains::interval::abstract_value::intersect;
use mirsa_domains::nullptr::transfer::{get_tracked_value, is_tracked};
use mirsa_framework::reduction::MAX_RELATED;
use rustc_middle::mir::{LocalDecls, Place};
use rustc_middle::ty::TyCtxt;

pub fn reduce<'tcx>(
    tcx: TyCtxt<'tcx>,
    decls: &LocalDecls<'tcx>,
    state: &mut CombinedState<'tcx>,
    place: Place<'tcx>,
) -> bool {
    for other in state.symbolic.equivalent_places(place, MAX_RELATED) {
        if other == place {
            continue;
        }
        if let Some(value) = state
            .interval
            .tracked_interval_resolved(&state.symbolic, &other)
        {
            if !value.is_empty()
                && !refine::refine_integer_place(&mut state.interval, &state.symbolic, place, value)
            {
                return false;
            }
        }
        if let Some(value) = state
            .interval
            .tracked_float_interval_resolved(&state.symbolic, &other)
        {
            if !value.is_bottom()
                && !refine::refine_float_place(&mut state.interval, &state.symbolic, place, value)
            {
                return false;
            }
        }
        if let (Some(left), Some(right)) = (
            state.interval.tracked_len_resolved(&state.symbolic, place),
            state.interval.tracked_len_resolved(&state.symbolic, other),
        ) {
            if !left.is_empty() && !right.is_empty() {
                let common = intersect(&left, &right);
                if common.is_empty() {
                    return false;
                }
                state
                    .interval
                    .set_len_resolved(&state.symbolic, place, common);
            }
        }
        let ty = place.ty(decls, tcx).ty;
        if is_tracked(ty) {
            let value = get_tracked_value(&mut state.nullptr, &state.symbolic, other, ty);
            if value != mirsa_domains::nullptr::NullPtr::Bot
                && !refine::refine_nullptr_place(
                    &mut state.nullptr,
                    &state.symbolic,
                    place,
                    ty,
                    value,
                )
            {
                return false;
            }
        }
    }
    true
}
