use crate::reduced_product::CombinedState;
use mirsa_domains::allocation::{
    PointerValue,
    transfer::{eval_pointer_operand, offset_pointer},
};
use mirsa_domains::interval::transfer::eval_operand;
use mirsa_framework::{call_model::PointerOffset, reduction::MAX_RELATED};
use rustc_middle::{
    mir::{LocalDecls, Operand, Place},
    ty::TyCtxt,
};

pub(super) fn reduce<'tcx>(
    tcx: TyCtxt<'tcx>,
    decls: &LocalDecls<'tcx>,
    state: &mut CombinedState<'tcx>,
    destination: Place<'tcx>,
    base: &Operand<'tcx>,
    offset: Option<&(PointerOffset, Operand<'tcx>)>,
) -> bool {
    if !state
        .allocation
        .pointer_within_budget(&state.symbolic, destination, MAX_RELATED)
        || base.place().is_some_and(|p| {
            !state
                .allocation
                .pointer_within_budget(&state.symbolic, p, MAX_RELATED)
        })
    {
        return true;
    }
    let mut wanted = eval_pointer_operand(
        tcx,
        &state.symbolic,
        &state.allocation,
        base,
        decls,
        destination.ty(decls, tcx).ty,
    );
    if let Some((offset, count)) = offset {
        let count = eval_operand(tcx, decls, count, &state.symbolic, &mut state.interval);
        wanted = offset_pointer(&state.allocation, &wanted, *offset, count);
    }
    let old = state
        .allocation
        .pointer_value_resolved(&state.symbolic, destination);
    if old.is_bottom()
        || wanted.is_bottom()
        || old
            .targets()
            .any(|(a, _)| wanted.targets().any(|(b, _)| a != b && a.may_alias(b)))
    {
        return true;
    }
    let common = PointerValue::intersect_equal(&old, &wanted);
    if common.is_bottom() {
        return false;
    }
    state
        .allocation
        .set_pointer_resolved(&state.symbolic, destination, common);
    true
}
