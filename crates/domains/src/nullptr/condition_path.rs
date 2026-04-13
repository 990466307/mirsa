use rustc_middle::mir::*;
use rustc_middle::ty::{Ty, TyCtxt, TyKind};

use super::abstract_value::NullPtr;
use super::state::{NullPtrState, get_tracked_value, is_tracked, set_tracked_value};

fn refine_place_to<'tcx>(
    st: &mut NullPtrState<'tcx>,
    place: Place<'tcx>,
    ty: Ty<'tcx>,
    wanted: NullPtr,
) -> bool {
    if !is_tracked(ty) {
        return true;
    }
    let current = get_tracked_value(st, place, ty);
    let Some(refined) = (match (current, wanted) {
        (NullPtr::Bot, _) | (_, NullPtr::Bot) => None,
        (NullPtr::MaybeNull, x) | (x, NullPtr::MaybeNull) => Some(x),
        (x, y) if x == y => Some(x),
        _ => None,
    }) else {
        return false;
    };
    set_tracked_value(st, place, ty, refined);
    true
}

fn find_last_cmp_assign<'tcx>(
    body: &'tcx Body<'tcx>,
    bb: BasicBlock,
    target: Place<'tcx>,
) -> Option<(BinOp, Operand<'tcx>, Operand<'tcx>)> {
    for stmt in body.basic_blocks[bb].statements.iter().rev() {
        let StatementKind::Assign(assign) = &stmt.kind else {
            continue;
        };
        let (place, rvalue) = &**assign;
        if *place != target {
            continue;
        }
        return match rvalue {
            Rvalue::BinaryOp(op, ops) if matches!(op, BinOp::Eq | BinOp::Ne) => {
                let (left, right) = &**ops;
                Some((*op, left.clone(), right.clone()))
            }
            _ => None,
        };
    }
    None
}

fn operand_nullness<'tcx>(
    tcx: TyCtxt<'tcx>,
    local_decls: &'tcx LocalDecls<'tcx>,
    st: &NullPtrState<'tcx>,
    op: &Operand<'tcx>,
) -> Option<NullPtr> {
    match op {
        Operand::Copy(place) | Operand::Move(place) => {
            let ty = place.ty(local_decls, tcx).ty;
            if is_tracked(ty) {
                Some(get_tracked_value(st, *place, ty))
            } else {
                None
            }
        }
        Operand::Constant(c) => {
            let si = c.const_.try_to_scalar_int()?;
            if si.to_bits_unchecked() == 0 {
                Some(NullPtr::Null)
            } else {
                Some(NullPtr::NonNull)
            }
        }
    }
}

fn refine_cmp<'tcx>(
    tcx: TyCtxt<'tcx>,
    local_decls: &'tcx LocalDecls<'tcx>,
    st: &mut NullPtrState<'tcx>,
    op: BinOp,
    truth: bool,
    left: &Operand<'tcx>,
    right: &Operand<'tcx>,
) -> Option<()> {
    let equal = match op {
        BinOp::Eq => truth,
        BinOp::Ne => !truth,
        _ => return Some(()),
    };

    let try_refine_side =
        |st: &mut NullPtrState<'tcx>, candidate: &Operand<'tcx>, other_op: &Operand<'tcx>| {
            let (Operand::Copy(place) | Operand::Move(place)) = candidate else {
                return Some(());
            };
            let ty = place.ty(local_decls, tcx).ty;
            if !is_tracked(ty) {
                return Some(());
            }

            let Some(other) = operand_nullness(tcx, local_decls, st, other_op) else {
                return Some(());
            };
            let wanted = match (equal, other) {
                (true, NullPtr::Null) => Some(NullPtr::Null),
                (true, NullPtr::NonNull) => Some(NullPtr::NonNull),
                (false, NullPtr::Null) => Some(NullPtr::NonNull),
                _ => None,
            };
            let Some(wanted) = wanted else {
                return Some(());
            };
            if refine_place_to(st, *place, ty, wanted) {
                Some(())
            } else {
                None
            }
        };

    try_refine_side(st, left, right)?;
    try_refine_side(st, right, left)?;

    Some(())
}

pub fn refine_edge<'tcx>(
    tcx: TyCtxt<'tcx>,
    body: &'tcx Body<'tcx>,
    pred: BasicBlock,
    succ: BasicBlock,
    in_state: &NullPtrState<'tcx>,
) -> Option<NullPtrState<'tcx>> {
    let term = body.basic_blocks[pred].terminator.as_ref()?;
    match &term.kind {
        TerminatorKind::Goto { target } => {
            if *target == succ {
                Some(in_state.clone())
            } else {
                None
            }
        }
        TerminatorKind::SwitchInt { discr, targets } => {
            let TyKind::Bool = discr.ty(&body.local_decls, tcx).kind() else {
                return Some(in_state.clone());
            };
            let mut values_for_succ: Vec<u128> = Vec::new();
            let mut all_values: Vec<u128> = Vec::new();
            for (val, target) in targets.iter() {
                all_values.push(val);
                if target == succ {
                    values_for_succ.push(val);
                }
            }
            let is_otherwise = targets.otherwise() == succ;
            let truth = if values_for_succ.len() == 1 {
                match values_for_succ[0] {
                    0 => Some(false),
                    1 => Some(true),
                    _ => None,
                }
            } else if is_otherwise {
                let has0 = all_values.contains(&0);
                let has1 = all_values.contains(&1);
                if has0 && !has1 {
                    Some(true)
                } else if has1 && !has0 {
                    Some(false)
                } else {
                    None
                }
            } else {
                None
            };
            let Some(truth) = truth else {
                return Some(in_state.clone());
            };

            let mut st = in_state.clone();
            if let Operand::Copy(cond_place) | Operand::Move(cond_place) = discr {
                if let Some((op, left, right)) = find_last_cmp_assign(body, pred, *cond_place) {
                    refine_cmp(tcx, &body.local_decls, &mut st, op, truth, &left, &right)?;
                }
            }
            Some(st)
        }
        _ => Some(in_state.clone()),
    }
}
