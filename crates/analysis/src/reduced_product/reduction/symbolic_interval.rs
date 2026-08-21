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
            let current = state.read_interval_resolved(symbolic, place);
            if current.is_empty() || current.low != current.high {
                return true;
            }
            switch_value_to_i128(tcx, expr.ty(local_decls, tcx), *value) != Some(current.low)
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
    let Some(expr) = symbolic.expr_for_place(*place).cloned() else {
        return true;
    };
    match expr {
        SymbolicExpr::Cmp { op, left, right } => {
            refine_comparison(tcx, local_decls, state, symbolic, op, truth, &left, &right)
        }
        SymbolicExpr::Call { callee, args } => {
            let path = tcx.def_path_str(callee);
            if !path.ends_with("::is_empty") {
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
                right_singleton,
                true,
            ) || !refine_integer_lower(
                state,
                symbolic,
                local_decls,
                tcx,
                right,
                left_singleton,
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
                right_singleton,
                false,
            ) || !refine_integer_upper(
                state,
                symbolic,
                local_decls,
                tcx,
                right,
                left_singleton,
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
                right_singleton,
                false,
            ) || !refine_integer_lower(
                state,
                symbolic,
                local_decls,
                tcx,
                right,
                left_singleton,
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
                right_singleton,
                true,
            ) || !refine_integer_upper(
                state,
                symbolic,
                local_decls,
                tcx,
                right,
                left_singleton,
                true,
            ) {
                return false;
            }
        }
        _ => {}
    }
    true
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

fn refine_integer_place<'tcx>(
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
    for other in state.interval_places() {
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

fn refine_float_place<'tcx>(
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
    for other in state.float_interval_places() {
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
