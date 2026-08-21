use mirsa_domains::allocation::AllocationState;
use mirsa_domains::allocation::state::is_allocation_pointer;
use mirsa_relations::symbolic::{SymbolicExpr, SymbolicFact, SymbolicState};
use rustc_middle::mir::{BinOp, LocalDecls, Operand};
use rustc_middle::ty::{TyCtxt, TyKind};

pub fn reduce_fact<'tcx>(
    tcx: TyCtxt<'tcx>,
    local_decls: &LocalDecls<'tcx>,
    state: &mut AllocationState<'tcx>,
    symbolic: &SymbolicState<'tcx>,
    fact: &SymbolicFact<'tcx>,
) -> bool {
    let (operand, truth) = match fact {
        SymbolicFact::EqConst { expr, value } => {
            let Some(value) = boolean_value(expr, *value, local_decls, tcx) else {
                return true;
            };
            (expr, value)
        }
        SymbolicFact::NeConst { expr, value } => {
            let Some(value) = boolean_value(expr, *value, local_decls, tcx) else {
                return true;
            };
            (expr, !value)
        }
    };
    let (Operand::Copy(place) | Operand::Move(place)) = operand else {
        return true;
    };
    let Some(SymbolicExpr::Cmp { op, left, right }) = symbolic.expr_for_place(*place) else {
        return true;
    };
    let equality_holds = matches!((op, truth), (BinOp::Eq, true) | (BinOp::Ne, false));
    if !equality_holds
        || !is_allocation_pointer(tcx, left.ty(local_decls, tcx))
        || !is_allocation_pointer(tcx, right.ty(local_decls, tcx))
    {
        return true;
    }
    let (Operand::Copy(left) | Operand::Move(left)) = left else {
        return true;
    };
    let (Operand::Copy(right) | Operand::Move(right)) = right else {
        return true;
    };
    state.constrain_equal_paths(symbolic, *left, *right)
}

fn boolean_value<'tcx>(
    operand: &Operand<'tcx>,
    value: u128,
    local_decls: &LocalDecls<'tcx>,
    tcx: TyCtxt<'tcx>,
) -> Option<bool> {
    if !matches!(operand.ty(local_decls, tcx).kind(), TyKind::Bool) {
        return None;
    }
    match value {
        0 => Some(false),
        1 => Some(true),
        _ => None,
    }
}
