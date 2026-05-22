use rustc_middle::mir::*;
use rustc_middle::ty::{Ty, TyCtxt, TyKind};

use super::abstract_value::NullPtr;
use super::access_path::AccessPath;
use super::state::{NullPtrState, get_tracked_value, is_tracked};
use super::transfer::const_nullness;

fn meet_nullptr(current: NullPtr, wanted: NullPtr) -> Option<NullPtr> {
    match (current, wanted) {
        (NullPtr::Bot, _) | (_, NullPtr::Bot) => None,
        (NullPtr::MaybeNull, x) | (x, NullPtr::MaybeNull) => Some(x),
        (x, y) if x == y => Some(x),
        _ => None,
    }
}

fn refine_tracked_fact<'tcx>(
    st: &mut NullPtrState<'tcx>,
    path: AccessPath,
    refined: NullPtr,
) -> bool {
    st.set_path(path.clone(), refined);
    let all_paths: Vec<AccessPath> = st.fact_paths().collect();
    for other in all_paths {
        if other == path || !st.eq.equiv_readonly(path.clone(), other.clone()) {
            continue;
        }
        let other_current = st.get_path(&other);
        let Some(other_refined) = meet_nullptr(other_current, refined) else {
            st.debug(format_args!("eq-kill {other}"));
            st.eq.kill(other);
            continue;
        };
        st.set_path(other, other_refined);
    }
    true
}

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
    let Some(refined) = meet_nullptr(current, wanted) else {
        return false;
    };
    let Some(path) = st.access_path_for_place(place) else {
        return false;
    };
    refine_tracked_fact(st, path, refined)
}

fn find_last_cmp_assign<'tcx>(
    body: &Body<'tcx>,
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

fn is_ptr_is_null_path(path: &str) -> bool {
    path.ends_with("::is_null") && path.contains("::ptr::")
}

enum BoolDef<'tcx> {
    Cmp(BinOp, Operand<'tcx>, Operand<'tcx>),
    IsNull(Operand<'tcx>),
}

fn find_bool_def<'tcx>(
    tcx: TyCtxt<'tcx>,
    body: &Body<'tcx>,
    bb: BasicBlock,
    target: Place<'tcx>,
) -> Option<BoolDef<'tcx>> {
    if let Some((op, left, right)) = find_last_cmp_assign(body, bb, target) {
        return Some(BoolDef::Cmp(op, left, right));
    }

    let mut matches = body
        .basic_blocks
        .iter_enumerated()
        .filter_map(|(_, bbdata)| {
            let term = bbdata.terminator.as_ref()?;
            let TerminatorKind::Call {
                func,
                args,
                destination,
                target: Some(call_target),
                ..
            } = &term.kind
            else {
                return None;
            };
            if *call_target != bb || *destination != target {
                return None;
            }
            let TyKind::FnDef(def_id, _) = func.ty(&body.local_decls, tcx).kind() else {
                return None;
            };
            let path = tcx.def_path_str(*def_id);
            if !is_ptr_is_null_path(&path) {
                return None;
            }
            Some(BoolDef::IsNull(args.first()?.node.clone()))
        });

    let first = matches.next()?;
    if matches.next().is_some() {
        return None;
    }
    Some(first)
}

fn operand_nullness<'tcx>(
    tcx: TyCtxt<'tcx>,
    local_decls: &LocalDecls<'tcx>,
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
        Operand::Constant(c) => const_nullness(tcx, c),
    }
}

fn refine_cmp<'tcx>(
    tcx: TyCtxt<'tcx>,
    local_decls: &LocalDecls<'tcx>,
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

    if equal {
        if let (Operand::Copy(pl) | Operand::Move(pl), Operand::Copy(pr) | Operand::Move(pr)) =
            (left, right)
        {
            let left_ty = pl.ty(local_decls, tcx).ty;
            let right_ty = pr.ty(local_decls, tcx).ty;
            if is_tracked(left_ty) && is_tracked(right_ty) {
                if let (Some(left_path), Some(right_path)) =
                    (st.access_path_for_place(*pl), st.access_path_for_place(*pr))
                {
                    st.debug(format_args!("eq {left_path} == {right_path}"));
                    st.eq.union(left_path, right_path);
                }
            }
        }
    }

    Some(())
}

fn refine_is_null<'tcx>(
    tcx: TyCtxt<'tcx>,
    local_decls: &LocalDecls<'tcx>,
    st: &mut NullPtrState<'tcx>,
    truth: bool,
    arg: &Operand<'tcx>,
) -> Option<()> {
    let (Operand::Copy(place) | Operand::Move(place)) = arg else {
        return Some(());
    };
    let ty = place.ty(local_decls, tcx).ty;
    if !is_tracked(ty) {
        return Some(());
    }

    let wanted = if truth {
        NullPtr::Null
    } else {
        NullPtr::NonNull
    };
    if refine_place_to(st, *place, ty, wanted) {
        Some(())
    } else {
        None
    }
}

pub fn refine_edge<'tcx>(
    tcx: TyCtxt<'tcx>,
    body: &Body<'tcx>,
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
                if let Some(def) = find_bool_def(tcx, body, pred, *cond_place) {
                    match def {
                        BoolDef::Cmp(op, left, right) => {
                            refine_cmp(tcx, &body.local_decls, &mut st, op, truth, &left, &right)?
                        }
                        BoolDef::IsNull(arg) => {
                            refine_is_null(tcx, &body.local_decls, &mut st, truth, &arg)?
                        }
                    }
                }
            }
            Some(st)
        }
        _ => Some(in_state.clone()),
    }
}
