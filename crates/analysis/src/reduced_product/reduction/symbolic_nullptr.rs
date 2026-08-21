use mirsa_domains::nullptr::abstract_value::NullPtr;
use mirsa_domains::nullptr::constraint::constrain;
use mirsa_domains::nullptr::state::NullPtrState;
use mirsa_domains::nullptr::transfer::{const_nullness, get_tracked_value, is_tracked};
use mirsa_framework::access_path::AccessPath;
use mirsa_relations::symbolic::{SymbolicExpr, SymbolicFact, SymbolicState};
use rustc_middle::mir::{BinOp, LocalDecls, Operand, Place};
use rustc_middle::ty::{Ty, TyCtxt, TyKind};

/// Refine nullness facts from a generic branch fact.  API recognition and
/// nullness lattice operations are deliberately owned by the nullptr domain.
pub fn reduce_fact<'tcx>(
    tcx: TyCtxt<'tcx>,
    local_decls: &LocalDecls<'tcx>,
    state: &mut NullPtrState<'tcx>,
    symbolic: &mut SymbolicState<'tcx>,
    fact: &SymbolicFact<'tcx>,
) -> bool {
    let (expr, truth) = match fact {
        SymbolicFact::EqConst { expr, value } => {
            let Some(truth) = bool_fact_truth(local_decls, tcx, expr, *value, true) else {
                return true;
            };
            (expr, truth)
        }
        SymbolicFact::NeConst { expr, value } => {
            let Some(truth) = bool_fact_truth(local_decls, tcx, expr, *value, false) else {
                return true;
            };
            (expr, truth)
        }
    };

    let (Operand::Copy(place) | Operand::Move(place)) = expr else {
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
            if !(path.ends_with("::is_null") && path.contains("::ptr::")) {
                return true;
            }
            args.first()
                .is_none_or(|arg| refine_is_null(tcx, local_decls, state, symbolic, truth, arg))
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

fn refine_comparison<'tcx>(
    tcx: TyCtxt<'tcx>,
    local_decls: &LocalDecls<'tcx>,
    state: &mut NullPtrState<'tcx>,
    symbolic: &mut SymbolicState<'tcx>,
    op: BinOp,
    truth: bool,
    left: &Operand<'tcx>,
    right: &Operand<'tcx>,
) -> bool {
    let equal = match op {
        BinOp::Eq => truth,
        BinOp::Ne => !truth,
        _ => return true,
    };
    if !refine_side(tcx, local_decls, state, symbolic, equal, left, right)
        || !refine_side(tcx, local_decls, state, symbolic, equal, right, left)
    {
        return false;
    }

    if equal {
        if let (
            Operand::Copy(left) | Operand::Move(left),
            Operand::Copy(right) | Operand::Move(right),
        ) = (left, right)
        {
            if is_tracked(left.ty(local_decls, tcx).ty) && is_tracked(right.ty(local_decls, tcx).ty)
            {
                if let (Some(left_path), Some(right_path)) = (
                    state.access_path_for_place_resolved(symbolic, *left),
                    state.access_path_for_place_resolved(symbolic, *right),
                ) {
                    state.debug(format_args!("eq {left_path} == {right_path}"));
                    symbolic.eq.union(left_path, right_path);
                }
            }
        }
    }
    true
}

fn refine_side<'tcx>(
    tcx: TyCtxt<'tcx>,
    local_decls: &LocalDecls<'tcx>,
    state: &mut NullPtrState<'tcx>,
    symbolic: &SymbolicState<'tcx>,
    equal: bool,
    candidate: &Operand<'tcx>,
    other: &Operand<'tcx>,
) -> bool {
    let (Operand::Copy(place) | Operand::Move(place)) = candidate else {
        return true;
    };
    let ty = place.ty(local_decls, tcx).ty;
    if !is_tracked(ty) {
        return true;
    }
    let Some(other) = operand_nullness(tcx, local_decls, state, symbolic, other) else {
        return true;
    };
    let wanted = match (equal, other) {
        (true, NullPtr::Null) => Some(NullPtr::Null),
        (true, NullPtr::NonNull) => Some(NullPtr::NonNull),
        (false, NullPtr::Null) => Some(NullPtr::NonNull),
        _ => None,
    };
    wanted.is_none_or(|wanted| refine_place(state, symbolic, *place, ty, wanted))
}

fn operand_nullness<'tcx>(
    tcx: TyCtxt<'tcx>,
    local_decls: &LocalDecls<'tcx>,
    state: &mut NullPtrState<'tcx>,
    symbolic: &SymbolicState<'tcx>,
    operand: &Operand<'tcx>,
) -> Option<NullPtr> {
    match operand {
        Operand::Copy(place) | Operand::Move(place) => {
            let ty = place.ty(local_decls, tcx).ty;
            is_tracked(ty).then(|| get_tracked_value(state, symbolic, *place, ty))
        }
        Operand::Constant(constant) => const_nullness(tcx, constant),
    }
}

fn refine_is_null<'tcx>(
    tcx: TyCtxt<'tcx>,
    local_decls: &LocalDecls<'tcx>,
    state: &mut NullPtrState<'tcx>,
    symbolic: &SymbolicState<'tcx>,
    truth: bool,
    arg: &Operand<'tcx>,
) -> bool {
    let (Operand::Copy(place) | Operand::Move(place)) = arg else {
        return true;
    };
    let ty = place.ty(local_decls, tcx).ty;
    let wanted = if truth {
        NullPtr::Null
    } else {
        NullPtr::NonNull
    };
    refine_place(state, symbolic, *place, ty, wanted)
}

fn refine_place<'tcx>(
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
    let paths: Vec<_> = state.fact_paths().collect();
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
