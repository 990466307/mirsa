use super::refine::{refine_float_place, refine_integer_place};
use mirsa_domains::interval::abstract_value::intersect;
use mirsa_domains::interval::float_interval::{intersect as intersect_float, next_down, next_up};
use mirsa_domains::interval::state::IntervalState;
use mirsa_domains::interval::transfer::{
    eval_float_operand, eval_operand, float_kind_for_ty, switch_value_to_i128,
};
use mirsa_domains::interval::{FloatInterval, Interval};
use mirsa_relations::symbolic::{SymbolicExpr, SymbolicFact, SymbolicState};
use rustc_middle::mir::{BinOp, LocalDecls, Operand, Place};
use rustc_middle::ty::{TyCtxt, TyKind};

/// Refine the interval domain with one generic path fact produced by the
/// symbolic relation layer.  All interval-specific interpretation stays here.
pub fn reduce_fact<'tcx>(
    tcx: TyCtxt<'tcx>,
    local_decls: &LocalDecls<'tcx>,
    state: &mut IntervalState<'tcx>,
    symbolic: &mut SymbolicState<'tcx>,
    fact: &SymbolicFact<'tcx>,
) -> bool {
    let (SymbolicFact::EqConst { expr, .. } | SymbolicFact::NeConst { expr, .. }) = fact;
    if !constrain_integer_type(tcx, local_decls, state, symbolic, expr) {
        return false;
    }
    match fact {
        SymbolicFact::EqConst { expr, value } => {
            let expr_ty = expr.ty(local_decls, tcx);
            if let (Some(place), Some(value)) = (
                integer_place_operand(local_decls, tcx, expr),
                switch_value_to_i128(tcx, expr_ty, *value),
            ) {
                if !refine_integer_place(state, symbolic, place, Interval::new(value, value)) {
                    return false;
                }
            }
            match bool_fact_truth(local_decls, tcx, expr, *value, true) {
                Some(truth) => refine_bool_expr(tcx, local_decls, state, symbolic, expr, truth),
                None => true,
            }
        }
        SymbolicFact::NeConst { expr, value } => {
            if let Some(truth) = bool_fact_truth(local_decls, tcx, expr, *value, false) {
                if !refine_bool_expr(tcx, local_decls, state, symbolic, expr, truth) {
                    return false;
                }
            }

            let Some(place) = integer_place_operand(local_decls, tcx, expr) else {
                return true;
            };
            let Some(value) = switch_value_to_i128(tcx, expr.ty(local_decls, tcx), *value) else {
                return true;
            };
            exclude_integer_value(state, symbolic, place, value)
        }
    }
}

fn bool_fact_truth<'tcx>(
    local_decls: &LocalDecls<'tcx>,
    tcx: TyCtxt<'tcx>,
    expr: &Operand<'tcx>,
    value: u128,
    equal: bool,
) -> Option<bool> {
    if !matches!(expr.ty(local_decls, tcx).kind(), TyKind::Bool) {
        return None;
    }
    let value = match value {
        0 => false,
        1 => true,
        _ => return None,
    };
    Some(if equal { value } else { !value })
}

fn refine_bool_expr<'tcx>(
    tcx: TyCtxt<'tcx>,
    local_decls: &LocalDecls<'tcx>,
    state: &mut IntervalState<'tcx>,
    symbolic: &mut SymbolicState<'tcx>,
    discr: &Operand<'tcx>,
    truth: bool,
) -> bool {
    let (Operand::Copy(place) | Operand::Move(place)) = discr else {
        return true;
    };
    let Some(expr) = super::expression(symbolic, *place).cloned() else {
        return true;
    };
    match expr {
        SymbolicExpr::Pointer { .. }
        | SymbolicExpr::PointerFromAddress { .. }
        | SymbolicExpr::Value(_)
        | SymbolicExpr::Address { .. }
        | SymbolicExpr::Length { .. } => true,
        SymbolicExpr::Cmp { op, left, right } => {
            refine_comparison(tcx, local_decls, state, symbolic, op, truth, &left, &right)
        }
        SymbolicExpr::Call { callee, args } => {
            let length_method = tcx.impl_of_method(callee).is_some_and(|id| {
                let ty = tcx.type_of(id).instantiate_identity();
                tcx.trait_id_of_impl(id).is_none()
                    && match ty.kind() {
                        TyKind::Slice(_) | TyKind::Str => true,
                        TyKind::Adt(adt, _) => ["Vec", "String"].iter().any(|name| {
                            tcx.is_diagnostic_item(rustc_span::Symbol::intern(name), adt.did())
                        }),
                        _ => false,
                    }
            });
            if tcx.item_name(callee).as_str() != "is_empty" || !length_method {
                return true;
            }
            args.first()
                .is_none_or(|receiver| refine_is_empty(state, symbolic, truth, receiver))
        }
    }
}

fn refine_comparison<'tcx>(
    tcx: TyCtxt<'tcx>,
    local_decls: &LocalDecls<'tcx>,
    state: &mut IntervalState<'tcx>,
    symbolic: &mut SymbolicState<'tcx>,
    op: BinOp,
    truth: bool,
    left: &Operand<'tcx>,
    right: &Operand<'tcx>,
) -> bool {
    if float_operand(local_decls, tcx, left) && float_operand(local_decls, tcx, right) {
        refine_float_comparison(tcx, local_decls, state, symbolic, op, truth, left, right)
    } else if integer_operand(local_decls, tcx, left) && integer_operand(local_decls, tcx, right) {
        refine_integer_comparison(tcx, local_decls, state, symbolic, op, truth, left, right)
    } else {
        true
    }
}

fn refine_integer_comparison<'tcx>(
    tcx: TyCtxt<'tcx>,
    local_decls: &LocalDecls<'tcx>,
    state: &mut IntervalState<'tcx>,
    symbolic: &mut SymbolicState<'tcx>,
    op: BinOp,
    truth: bool,
    left: &Operand<'tcx>,
    right: &Operand<'tcx>,
) -> bool {
    if !symbolic.comparison_may_hold(tcx, local_decls, op, truth, left, right)
        || !constrain_integer_type(tcx, local_decls, state, symbolic, left)
        || !constrain_integer_type(tcx, local_decls, state, symbolic, right)
    {
        return false;
    }
    let left_iv = eval_operand(tcx, local_decls, left, symbolic, state);
    let right_iv = eval_operand(tcx, local_decls, right, symbolic, state);
    state.debug(format_args!(
        "reduce cmp {:?} truth={} left={} right={}",
        op, truth, left_iv, right_iv
    ));

    let left_singleton = singleton(left_iv);
    let right_singleton = singleton(right_iv);
    match (op, truth) {
        (BinOp::Eq, true) | (BinOp::Ne, false) => {
            let common = intersect(&left_iv, &right_iv);
            if common.is_empty() && !left_iv.is_empty() && !right_iv.is_empty() {
                return false;
            }
            if let Some(place) = integer_place_operand(local_decls, tcx, left) {
                if !refine_integer_place(state, symbolic, place, common) {
                    return false;
                }
            }
            if let Some(place) = integer_place_operand(local_decls, tcx, right) {
                if !refine_integer_place(state, symbolic, place, common) {
                    return false;
                }
            }
            if let (Some(left), Some(right)) = (
                integer_place_operand(local_decls, tcx, left),
                integer_place_operand(local_decls, tcx, right),
            ) {
                symbolic.union_places(left, right);
            }
        }
        (BinOp::Eq, false) | (BinOp::Ne, true) => {
            for (operand, excluded) in [(left, right_singleton), (right, left_singleton)] {
                if let (Some(place), Some(value)) =
                    (integer_place_operand(local_decls, tcx, operand), excluded)
                {
                    if !exclude_integer_value(state, symbolic, place, value) {
                        return false;
                    }
                }
            }
            if left_singleton.is_some()
                && left_singleton == right_singleton
                && !left_iv.is_empty()
                && !right_iv.is_empty()
            {
                return false;
            }
        }
        (BinOp::Lt, true) | (BinOp::Ge, false) => {
            if !refine_integer_upper(
                state,
                symbolic,
                local_decls,
                tcx,
                left,
                finite_upper(right_iv),
                true,
            ) || !refine_integer_lower(
                state,
                symbolic,
                local_decls,
                tcx,
                right,
                finite_lower(left_iv),
                true,
            ) {
                return false;
            }
        }
        (BinOp::Lt, false) | (BinOp::Ge, true) => {
            if !refine_integer_lower(
                state,
                symbolic,
                local_decls,
                tcx,
                left,
                finite_lower(right_iv),
                false,
            ) || !refine_integer_upper(
                state,
                symbolic,
                local_decls,
                tcx,
                right,
                finite_upper(left_iv),
                false,
            ) {
                return false;
            }
        }
        (BinOp::Le, true) | (BinOp::Gt, false) => {
            if !refine_integer_upper(
                state,
                symbolic,
                local_decls,
                tcx,
                left,
                finite_upper(right_iv),
                false,
            ) || !refine_integer_lower(
                state,
                symbolic,
                local_decls,
                tcx,
                right,
                finite_lower(left_iv),
                false,
            ) {
                return false;
            }
        }
        (BinOp::Le, false) | (BinOp::Gt, true) => {
            if !refine_integer_lower(
                state,
                symbolic,
                local_decls,
                tcx,
                left,
                finite_lower(right_iv),
                true,
            ) || !refine_integer_upper(
                state,
                symbolic,
                local_decls,
                tcx,
                right,
                finite_upper(left_iv),
                true,
            ) {
                return false;
            }
        }
        _ => {}
    }
    true
}

fn finite_upper(value: Interval) -> Option<i128> {
    (!value.is_empty() && value.high != i128::MAX).then_some(value.high)
}

fn finite_lower(value: Interval) -> Option<i128> {
    (!value.is_empty() && value.low != i128::MIN).then_some(value.low)
}

fn singleton(value: Interval) -> Option<i128> {
    (!value.is_empty() && value.low == value.high).then_some(value.low)
}

fn refine_integer_upper<'tcx>(
    state: &mut IntervalState<'tcx>,
    symbolic: &SymbolicState<'tcx>,
    local_decls: &LocalDecls<'tcx>,
    tcx: TyCtxt<'tcx>,
    operand: &Operand<'tcx>,
    bound: Option<i128>,
    strict: bool,
) -> bool {
    let (Some(place), Some(bound)) = (integer_place_operand(local_decls, tcx, operand), bound)
    else {
        return true;
    };
    let high = if strict {
        bound.saturating_sub(1)
    } else {
        bound
    };
    refine_integer_place(state, symbolic, place, Interval::new(i128::MIN, high))
}

fn refine_integer_lower<'tcx>(
    state: &mut IntervalState<'tcx>,
    symbolic: &SymbolicState<'tcx>,
    local_decls: &LocalDecls<'tcx>,
    tcx: TyCtxt<'tcx>,
    operand: &Operand<'tcx>,
    bound: Option<i128>,
    strict: bool,
) -> bool {
    let (Some(place), Some(bound)) = (integer_place_operand(local_decls, tcx, operand), bound)
    else {
        return true;
    };
    let low = if strict {
        bound.saturating_add(1)
    } else {
        bound
    };
    refine_integer_place(state, symbolic, place, Interval::new(low, i128::MAX))
}

fn refine_float_comparison<'tcx>(
    tcx: TyCtxt<'tcx>,
    local_decls: &LocalDecls<'tcx>,
    state: &mut IntervalState<'tcx>,
    symbolic: &mut SymbolicState<'tcx>,
    op: BinOp,
    truth: bool,
    left: &Operand<'tcx>,
    right: &Operand<'tcx>,
) -> bool {
    let Some(kind) = float_kind_for_ty(left.ty(local_decls, tcx)) else {
        return true;
    };
    let left_iv = eval_float_operand(tcx, local_decls, left, symbolic, state);
    let right_iv = eval_float_operand(tcx, local_decls, right, symbolic, state);
    state.debug(format_args!(
        "reduce float cmp {:?} truth={} left={} right={}",
        op, truth, left_iv, right_iv
    ));

    match (op, truth) {
        (BinOp::Eq, true) | (BinOp::Ne, false) => {
            let common = intersect_float(&left_iv.without_nan(), &right_iv.without_nan());
            if common.is_bottom() {
                return false;
            }
            if let Some(place) = float_place_operand(local_decls, tcx, left) {
                if !refine_float_place(state, symbolic, place, common) {
                    return false;
                }
            }
            if let Some(place) = float_place_operand(local_decls, tcx, right) {
                if !refine_float_place(state, symbolic, place, common) {
                    return false;
                }
            }
            if let (Some(left), Some(right)) = (
                float_place_operand(local_decls, tcx, left),
                float_place_operand(local_decls, tcx, right),
            ) {
                symbolic.union_places(left, right);
            }
            return true;
        }
        (BinOp::Eq, false) | (BinOp::Ne, true) => {
            return left_iv.may_nan
                || right_iv.may_nan
                || !left_iv.is_numeric_singleton()
                || !right_iv.is_numeric_singleton()
                || left_iv.low != right_iv.low;
        }
        _ => {}
    }

    if !matches!(op, BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge) {
        return true;
    }
    if !truth && (left_iv.may_nan || right_iv.may_nan) {
        return true;
    }
    let (relation, left_iv, right_iv) = match (op, truth) {
        (BinOp::Lt, true) | (BinOp::Ge, false) => {
            (BinOp::Lt, left_iv.without_nan(), right_iv.without_nan())
        }
        (BinOp::Le, true) | (BinOp::Gt, false) => {
            (BinOp::Le, left_iv.without_nan(), right_iv.without_nan())
        }
        (BinOp::Gt, true) | (BinOp::Le, false) => {
            (BinOp::Gt, left_iv.without_nan(), right_iv.without_nan())
        }
        (BinOp::Ge, true) | (BinOp::Lt, false) => {
            (BinOp::Ge, left_iv.without_nan(), right_iv.without_nan())
        }
        _ => return true,
    };
    if !left_iv.has_numeric_values() || !right_iv.has_numeric_values() {
        return false;
    }

    let (left_wanted, right_wanted) = match relation {
        BinOp::Lt => (
            FloatInterval::numeric(f64::NEG_INFINITY, next_down(kind, right_iv.high)),
            FloatInterval::numeric(next_up(kind, left_iv.low), f64::INFINITY),
        ),
        BinOp::Le => (
            FloatInterval::numeric(f64::NEG_INFINITY, right_iv.high),
            FloatInterval::numeric(left_iv.low, f64::INFINITY),
        ),
        BinOp::Gt => (
            FloatInterval::numeric(next_up(kind, right_iv.low), f64::INFINITY),
            FloatInterval::numeric(f64::NEG_INFINITY, next_down(kind, left_iv.high)),
        ),
        BinOp::Ge => (
            FloatInterval::numeric(right_iv.low, f64::INFINITY),
            FloatInterval::numeric(f64::NEG_INFINITY, left_iv.high),
        ),
        _ => unreachable!(),
    };
    if let Some(place) = float_place_operand(local_decls, tcx, left) {
        if !refine_float_place(state, symbolic, place, left_wanted) {
            return false;
        }
    }
    if let Some(place) = float_place_operand(local_decls, tcx, right) {
        if !refine_float_place(state, symbolic, place, right_wanted) {
            return false;
        }
    }
    true
}

fn refine_is_empty<'tcx>(
    state: &mut IntervalState<'tcx>,
    symbolic: &SymbolicState<'tcx>,
    truth: bool,
    receiver: &Operand<'tcx>,
) -> bool {
    let (Operand::Copy(place) | Operand::Move(place)) = receiver else {
        return true;
    };
    let current = state.read_len_resolved_or_top(symbolic, *place);
    let wanted = if truth {
        Interval::new(0, 0)
    } else {
        Interval::new(1, i128::MAX)
    };
    let refined = intersect(&current, &wanted);
    if refined.is_empty() {
        return false;
    }
    state.debug(format_args!(
        "reduce is_empty truth={} {:?}.len: {} ∩ {} = {}",
        truth, place, current, wanted, refined
    ));
    state.set_len_resolved(symbolic, *place, refined);
    true
}

fn is_integer_scalar(ty: rustc_middle::ty::Ty<'_>) -> bool {
    matches!(
        ty.kind(),
        TyKind::Int(_) | TyKind::Uint(_) | TyKind::Bool | TyKind::Char
    )
}

fn integer_operand<'tcx>(
    local_decls: &LocalDecls<'tcx>,
    tcx: TyCtxt<'tcx>,
    operand: &Operand<'tcx>,
) -> bool {
    is_integer_scalar(operand.ty(local_decls, tcx))
}

fn integer_place_operand<'tcx>(
    local_decls: &LocalDecls<'tcx>,
    tcx: TyCtxt<'tcx>,
    operand: &Operand<'tcx>,
) -> Option<Place<'tcx>> {
    let (Operand::Copy(place) | Operand::Move(place)) = operand else {
        return None;
    };
    is_integer_scalar(place.ty(local_decls, tcx).ty).then_some(*place)
}

fn float_operand<'tcx>(
    local_decls: &LocalDecls<'tcx>,
    tcx: TyCtxt<'tcx>,
    operand: &Operand<'tcx>,
) -> bool {
    float_kind_for_ty(operand.ty(local_decls, tcx)).is_some()
}

fn float_place_operand<'tcx>(
    local_decls: &LocalDecls<'tcx>,
    tcx: TyCtxt<'tcx>,
    operand: &Operand<'tcx>,
) -> Option<Place<'tcx>> {
    let (Operand::Copy(place) | Operand::Move(place)) = operand else {
        return None;
    };
    float_kind_for_ty(place.ty(local_decls, tcx).ty).map(|_| *place)
}

fn constrain_integer_type<'tcx>(
    tcx: TyCtxt<'tcx>,
    local_decls: &LocalDecls<'tcx>,
    state: &mut IntervalState<'tcx>,
    symbolic: &SymbolicState<'tcx>,
    operand: &Operand<'tcx>,
) -> bool {
    let Some(place) = integer_place_operand(local_decls, tcx, operand) else {
        return true;
    };
    let ty = operand.ty(local_decls, tcx);
    let bounds = match ty.kind() {
        TyKind::Uint(kind) => {
            let bits = kind
                .bit_width()
                .unwrap_or(tcx.data_layout.pointer_size.bits());
            if bits == 128 {
                return true;
            }
            Interval::new(0, (1_i128 << bits) - 1)
        }
        TyKind::Int(kind) => {
            let bits = kind
                .bit_width()
                .unwrap_or(tcx.data_layout.pointer_size.bits());
            if bits == 128 {
                return true;
            }
            Interval::new(-(1_i128 << (bits - 1)), (1_i128 << (bits - 1)) - 1)
        }
        TyKind::Bool => Interval::new(0, 1),
        TyKind::Char => Interval::new(0, 0x10ffff),
        _ => return true,
    };
    refine_integer_place(state, symbolic, place, bounds)
}

fn exclude_integer_value<'tcx>(
    state: &mut IntervalState<'tcx>,
    symbolic: &SymbolicState<'tcx>,
    place: Place<'tcx>,
    excluded: i128,
) -> bool {
    let mut value = state.read_interval_resolved(symbolic, place);
    if value.is_empty() {
        return true;
    }
    if value.low == excluded && value.high == excluded {
        return false;
    }
    if value.low == excluded {
        value.low += 1;
    }
    if value.high == excluded {
        value.high -= 1;
    }
    refine_integer_place(state, symbolic, place, value)
}
