use super::refine;
use crate::reduced_product::state::CombinedState;
use mirsa_domains::allocation::state::type_size;
use mirsa_domains::interval::abstract_value::{Interval, intersect};
use mirsa_domains::nullptr::NullPtr;
use mirsa_domains::nullptr::transfer::get_tracked_value;
use mirsa_framework::reduction::MAX_RELATED;
use mirsa_relations::symbolic::SymbolicExpr;
use rustc_middle::mir::{LocalDecls, Place};
use rustc_middle::ty::{TyCtxt, TyKind};

pub fn reduce<'tcx>(
    tcx: TyCtxt<'tcx>,
    local_decls: &LocalDecls<'tcx>,
    state: &mut CombinedState<'tcx>,
    destination: Place<'tcx>,
) -> bool {
    let bounds = Interval::new(0, (1_i128 << tcx.data_layout.pointer_size.bits()) - 1);
    if let Some(expr) = super::expression(&state.symbolic, destination).cloned() {
        match expr {
            SymbolicExpr::Pointer { base, offset } => {
                if !super::pointer_relations::reduce(
                    tcx,
                    local_decls,
                    state,
                    destination,
                    &base,
                    offset.as_ref(),
                ) {
                    return false;
                }
            }
            SymbolicExpr::Address { pointer } => {
                let Some(pointer) = pointer.place() else {
                    return true;
                };
                let ty = pointer.ty(local_decls, tcx).ty;
                if !matches!(ty.kind(), TyKind::RawPtr(..)) {
                    return true;
                }
                let nullness = get_tracked_value(&mut state.nullptr, &state.symbolic, pointer, ty);
                let wanted = match nullness {
                    NullPtr::Null => Interval::new(0, 0),
                    NullPtr::NonNull => Interval::new(1, bounds.high),
                    _ => bounds,
                };
                if !refine::refine_integer_place(
                    &mut state.interval,
                    &state.symbolic,
                    destination,
                    wanted,
                ) {
                    return false;
                }
                let value = state
                    .interval
                    .read_interval_resolved(&state.symbolic, destination);
                if value.is_empty() {
                    return true;
                }
                let wanted = if value.low > 0 {
                    NullPtr::NonNull
                } else if value.low == 0 && value.high == 0 {
                    NullPtr::Null
                } else {
                    return true;
                };
                if !refine::refine_nullptr_place(
                    &mut state.nullptr,
                    &state.symbolic,
                    pointer,
                    ty,
                    wanted,
                ) {
                    return false;
                }
            }
            SymbolicExpr::Length { place } => {
                let value = state
                    .interval
                    .read_interval_resolved(&state.symbolic, destination);
                if value.is_empty() {
                    return true;
                }
                let length = state
                    .interval
                    .read_len_resolved_or_top(&state.symbolic, place);
                if length.is_empty() {
                    return true;
                }
                let common = intersect(&intersect(&value, &length), &bounds);
                if common.is_empty() {
                    return false;
                }
                state
                    .interval
                    .set_len_resolved(&state.symbolic, place, common);
                if !refine::refine_integer_place(
                    &mut state.interval,
                    &state.symbolic,
                    destination,
                    common,
                ) {
                    return false;
                }
            }
            SymbolicExpr::Value(value) => {
                if !matches!(
                    destination.ty(local_decls, tcx).ty.kind(),
                    TyKind::Int(_) | TyKind::Uint(_) | TyKind::Bool | TyKind::Char
                ) {
                    return true;
                }
                let wanted = evaluate_value(tcx, local_decls, state, &value);
                if !refine::refine_integer_place(
                    &mut state.interval,
                    &state.symbolic,
                    destination,
                    wanted,
                ) {
                    return false;
                }
            }
            SymbolicExpr::Cmp { op, left, right } => {
                let rvalue = rustc_middle::mir::Rvalue::BinaryOp(op, Box::new((left, right)));
                let wanted = mirsa_domains::interval::transfer::eval_assign_rhs_interval(
                    tcx,
                    &mut state.interval,
                    &state.symbolic,
                    local_decls,
                    &rvalue,
                );
                if !refine::refine_integer_place(
                    &mut state.interval,
                    &state.symbolic,
                    destination,
                    wanted,
                ) {
                    return false;
                }
            }
            _ => {}
        }
    }
    reduce_slice_extent(tcx, local_decls, state, destination)
}

fn reduce_slice_extent<'tcx>(
    tcx: TyCtxt<'tcx>,
    local_decls: &LocalDecls<'tcx>,
    state: &mut CombinedState<'tcx>,
    place: Place<'tcx>,
) -> bool {
    let TyKind::Ref(_, inner, _) = place.ty(local_decls, tcx).ty.kind() else {
        return true;
    };
    let element_bytes = match inner.kind() {
        TyKind::Slice(element) => type_size(tcx, *element),
        TyKind::Str => Some(Interval::new(1, 1)),
        _ => None,
    };
    let Some(element_bytes) = element_bytes else {
        return true;
    };
    let Some(length) = state.interval.tracked_len_resolved(&state.symbolic, place) else {
        return true;
    };
    if length.is_empty() || length.low <= 0 || element_bytes.low <= 0 {
        return true;
    }
    let Some(bytes) = length.low.checked_mul(element_bytes.low) else {
        return true;
    };
    if !state
        .allocation
        .pointer_within_budget(&state.symbolic, place, MAX_RELATED)
    {
        return true;
    }
    let pointer = state
        .allocation
        .pointer_value_resolved(&state.symbolic, place);
    let allocated = pointer.with_allocation();
    if allocated.is_bottom() && !pointer.is_bottom() {
        return false;
    }
    if allocated != pointer {
        state
            .allocation
            .set_pointer_resolved(&state.symbolic, place, allocated.clone());
    }
    state.allocation.constrain_minimum_extent(&allocated, bytes)
}

fn evaluate_value<'tcx>(
    tcx: TyCtxt<'tcx>,
    local_decls: &LocalDecls<'tcx>,
    state: &mut CombinedState<'tcx>,
    value: &mirsa_relations::value_expr::ValueExpr<'tcx>,
) -> Interval {
    use mirsa_domains::interval::transfer::{
        eval_assign_rhs_interval, eval_checked_binary_interval,
    };
    use mirsa_relations::value_expr::{CheckedComponent, ValueExpr};
    use rustc_middle::mir::Rvalue;
    let rvalue = match value {
        ValueExpr::Use(operand) => Rvalue::Use(operand.clone()),
        ValueExpr::Binary { op, left, right } => {
            Rvalue::BinaryOp(*op, Box::new((left.clone(), right.clone())))
        }
        ValueExpr::Unary { op, operand } => Rvalue::UnaryOp(*op, operand.clone()),
        ValueExpr::Cast {
            kind,
            operand,
            target,
        } => Rvalue::Cast(*kind, operand.clone(), *target),
        ValueExpr::CheckedBinary {
            op,
            left,
            right,
            component,
        } => {
            let (value, overflow) = eval_checked_binary_interval(
                tcx,
                &mut state.interval,
                &state.symbolic,
                local_decls,
                op,
                left,
                right,
            );
            return match component {
                CheckedComponent::Value => value,
                CheckedComponent::Overflow => overflow,
            };
        }
    };
    eval_assign_rhs_interval(
        tcx,
        &mut state.interval,
        &state.symbolic,
        local_decls,
        &rvalue,
    )
}
