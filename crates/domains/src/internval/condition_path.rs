use rustc_middle::mir::*;
use rustc_middle::ty::{TyCtxt, TyKind};

use super::abstract_value::{Internval, intersect};
use super::state::{InternvalState, switch_value_to_i128};
use super::transfer::{eval_operand, internval_of_const};

// 将一个 place 与新区间求交，并传播到等价类中的 place。
fn refine_place_with_interval<'tcx>(
    st: &mut InternvalState<'tcx>,
    place: Place<'tcx>,
    new_iv: Internval,
) -> bool {
    let current = st.get_internval(&place);
    let refined = intersect(&current, &new_iv);
    if refined.is_empty() {
        return false;
    }
    st.set_internval(place, refined);

    let all_places: Vec<Place<'tcx>> = st.internval.keys().copied().collect();
    for other in all_places {
        if other == place {
            continue;
        }
        if st.eq.equiv_readonly(place, other) {
            let other_current = st.get_internval(&other);
            let other_refined = intersect(&other_current, &refined);
            if other_refined.is_empty() {
                // Contradiction means this equality is stale on this path.
                st.eq.kill(other);
                continue;
            }
            st.set_internval(other, other_refined);
        }
    }
    true
}

// 在同一基本块内查找目标 place 最近一次比较赋值。
fn find_last_cmp_assign<'tcx>(
    body: &Body<'tcx>,
    bb: BasicBlock,
    target: Place<'tcx>,
) -> Option<(BinOp, Operand<'tcx>, Operand<'tcx>)> {
    for stmt in body.basic_blocks[bb].statements.iter().rev() {
        if let StatementKind::Assign(assign) = &stmt.kind {
            let (place, rvalue) = &**assign;
            if *place != target {
                continue;
            }
            return match rvalue {
                Rvalue::BinaryOp(op, ops)
                    if matches!(
                        op,
                        BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge | BinOp::Eq | BinOp::Ne
                    ) =>
                {
                    let (a, b) = &**ops;
                    Some((*op, a.clone(), b.clone()))
                }
                // Found a newer assignment to `target` that is not a tracked comparison:
                // stop search to avoid using stale comparison result.
                _ => None,
            };
        }
    }
    None
}

fn is_slice_is_empty_path(path: &str) -> bool {
    path.ends_with("::is_empty")
}

enum BoolDef<'tcx> {
    Cmp(BinOp, Operand<'tcx>, Operand<'tcx>),
    IsEmpty(Operand<'tcx>),
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
            if !is_slice_is_empty_path(&path) {
                return None;
            }
            Some(BoolDef::IsEmpty(args.first()?.node.clone()))
        });

    let first = matches.next()?;
    if matches.next().is_some() {
        return None;
    }
    Some(first)
}

// 根据边上的比较真值细化参与比较的操作数区间。
fn refine_cmp<'tcx>(
    tcx: TyCtxt<'tcx>,
    local_decls: &LocalDecls<'tcx>,
    st: &mut InternvalState<'tcx>,
    op: BinOp,
    truth: bool,
    left: &Operand<'tcx>,
    right: &Operand<'tcx>,
) -> Option<()> {
    let left_iv = eval_operand(tcx, local_decls, left, st);
    let right_iv = eval_operand(tcx, local_decls, right, st);

    let left_singleton = if left_iv.low == left_iv.high {
        Some(left_iv.low)
    } else {
        None
    };
    let right_singleton = if right_iv.low == right_iv.high {
        Some(right_iv.low)
    } else {
        None
    };

    match (op, truth) {
        (BinOp::Eq, true) | (BinOp::Ne, false) => {
            let intersected = intersect(&left_iv, &right_iv);
            if intersected.is_empty() && !left_iv.is_empty() && !right_iv.is_empty() {
                return None;
            }
            if let Operand::Copy(p) | Operand::Move(p) = left {
                if !refine_place_with_interval(st, *p, intersected) {
                    return None;
                }
            }
            if let Operand::Copy(p) | Operand::Move(p) = right {
                if !refine_place_with_interval(st, *p, intersected) {
                    return None;
                }
            }
            if let (Operand::Copy(pl) | Operand::Move(pl), Operand::Copy(pr) | Operand::Move(pr)) =
                (left, right)
            {
                st.eq.union(*pl, *pr);
            }
        }
        (BinOp::Eq, false) | (BinOp::Ne, true) => {
            if left_singleton.is_some()
                && right_singleton.is_some()
                && left_singleton == right_singleton
            {
                return None;
            }
        }
        (BinOp::Lt, true) | (BinOp::Ge, false) => {
            if let (Some(c), Operand::Copy(p) | Operand::Move(p)) = (right_singleton, left) {
                if !refine_place_with_interval(
                    st,
                    *p,
                    Internval::new(i128::MIN, c.saturating_sub(1)),
                ) {
                    return None;
                }
            }
            if let (Some(c), Operand::Copy(p) | Operand::Move(p)) = (left_singleton, right) {
                if !refine_place_with_interval(
                    st,
                    *p,
                    Internval::new(c.saturating_add(1), i128::MAX),
                ) {
                    return None;
                }
            }
        }
        (BinOp::Lt, false) | (BinOp::Ge, true) => {
            if let (Some(c), Operand::Copy(p) | Operand::Move(p)) = (right_singleton, left) {
                if !refine_place_with_interval(st, *p, Internval::new(c, i128::MAX)) {
                    return None;
                }
            }
            if let (Some(c), Operand::Copy(p) | Operand::Move(p)) = (left_singleton, right) {
                if !refine_place_with_interval(st, *p, Internval::new(i128::MIN, c)) {
                    return None;
                }
            }
        }
        (BinOp::Le, true) | (BinOp::Gt, false) => {
            if let (Some(c), Operand::Copy(p) | Operand::Move(p)) = (right_singleton, left) {
                if !refine_place_with_interval(st, *p, Internval::new(i128::MIN, c)) {
                    return None;
                }
            }
            if let (Some(c), Operand::Copy(p) | Operand::Move(p)) = (left_singleton, right) {
                if !refine_place_with_interval(st, *p, Internval::new(c, i128::MAX)) {
                    return None;
                }
            }
        }
        (BinOp::Le, false) | (BinOp::Gt, true) => {
            if let (Some(c), Operand::Copy(p) | Operand::Move(p)) = (right_singleton, left) {
                if !refine_place_with_interval(
                    st,
                    *p,
                    Internval::new(c.saturating_add(1), i128::MAX),
                ) {
                    return None;
                }
            }
            if let (Some(c), Operand::Copy(p) | Operand::Move(p)) = (left_singleton, right) {
                if !refine_place_with_interval(
                    st,
                    *p,
                    Internval::new(i128::MIN, c.saturating_sub(1)),
                ) {
                    return None;
                }
            }
        }
        _ => {}
    }

    Some(())
}

fn refine_is_empty<'tcx>(
    st: &mut InternvalState<'tcx>,
    truth: bool,
    receiver: &Operand<'tcx>,
) -> Option<()> {
    let (Operand::Copy(place) | Operand::Move(place)) = receiver else {
        return Some(());
    };
    let current = st.get_slice_meta(place).unwrap_or_else(Internval::top);
    let wanted = if truth {
        Internval::new(0, 0)
    } else {
        Internval::new(1, i128::MAX)
    };
    let refined = intersect(&current, &wanted);
    if refined.is_empty() {
        return None;
    }
    st.set_slice_meta(*place, refined);
    Some(())
}

// 利用分支条件（Goto/SwitchInt）对边状态做细化。
pub fn refine_edge<'tcx>(
    tcx: TyCtxt<'tcx>,
    body: &Body<'tcx>,
    pred: BasicBlock,
    succ: BasicBlock,
    in_state: &InternvalState<'tcx>,
) -> Option<InternvalState<'tcx>> {
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
            let mut values_for_succ: Vec<u128> = Vec::new();
            let mut all_values: Vec<u128> = Vec::new();
            for (val, target) in targets.iter() {
                all_values.push(val);
                if target == succ {
                    values_for_succ.push(val);
                }
            }
            let is_otherwise = targets.otherwise() == succ;
            if values_for_succ.is_empty() && !is_otherwise {
                return None;
            }

            let discr_ty = discr.ty(&body.local_decls, tcx);
            let mut st = in_state.clone();

            match discr {
                Operand::Copy(place) | Operand::Move(place) => {
                    let current = st.get_internval(place);
                    let mut values_i128: Vec<i128> = Vec::new();
                    for v in &values_for_succ {
                        if let Some(i) = switch_value_to_i128(tcx, discr_ty, *v) {
                            values_i128.push(i);
                        }
                    }
                    let mut all_values_i128: Vec<i128> = Vec::new();
                    for v in &all_values {
                        if let Some(i) = switch_value_to_i128(tcx, discr_ty, *v) {
                            all_values_i128.push(i);
                        }
                    }

                    let discr_is_bool = matches!(discr_ty.kind(), TyKind::Bool);

                    if !values_i128.is_empty()
                        && values_i128.len() == 1
                        && !(discr_is_bool && current.is_empty())
                    {
                        let v = values_i128[0];
                        if !refine_place_with_interval(&mut st, *place, Internval::new(v, v)) {
                            return None;
                        }
                    } else if is_otherwise && current.low == current.high {
                        let v = current.low;
                        if all_values_i128.contains(&v) {
                            return None;
                        }
                    }

                    if discr_is_bool {
                        let mut branch_truth: Option<bool> = None;
                        if values_for_succ.len() == 1 {
                            branch_truth = match values_for_succ[0] {
                                0 => Some(false),
                                1 => Some(true),
                                _ => None,
                            };
                        } else if is_otherwise {
                            let has0 = all_values.contains(&0);
                            let has1 = all_values.contains(&1);
                            if has0 && has1 {
                                return None;
                            } else if has0 && !has1 {
                                branch_truth = Some(true);
                            } else if has1 && !has0 {
                                branch_truth = Some(false);
                            }
                        }

                        if let Some(truth) = branch_truth {
                            if let Some(def) = find_bool_def(tcx, body, pred, *place) {
                                match def {
                                    BoolDef::Cmp(op, left, right) => {
                                        if refine_cmp(
                                            tcx,
                                            &body.local_decls,
                                            &mut st,
                                            op,
                                            truth,
                                            &left,
                                            &right,
                                        )
                                        .is_none()
                                        {
                                            return None;
                                        }
                                    }
                                    BoolDef::IsEmpty(receiver) => {
                                        if refine_is_empty(&mut st, truth, &receiver).is_none() {
                                            return None;
                                        }
                                    }
                                }
                            }
                        }
                    }

                    Some(st)
                }
                Operand::Constant(c) => {
                    let c_iv = internval_of_const(c);
                    if c_iv.is_empty() || c_iv.low != c_iv.high {
                        return Some(st);
                    }
                    let c_val = c_iv.low;
                    let mut values_for_succ_i128: Vec<i128> = Vec::new();
                    for v in &values_for_succ {
                        if let Some(i) = switch_value_to_i128(tcx, discr_ty, *v) {
                            values_for_succ_i128.push(i);
                        }
                    }
                    let mut all_values_i128: Vec<i128> = Vec::new();
                    for v in &all_values {
                        if let Some(i) = switch_value_to_i128(tcx, discr_ty, *v) {
                            all_values_i128.push(i);
                        }
                    }
                    let matches_succ_value = values_for_succ_i128.contains(&c_val);
                    let matches_any_value = all_values_i128.contains(&c_val);
                    if matches_succ_value || (is_otherwise && !matches_any_value) {
                        Some(st)
                    } else {
                        None
                    }
                }
            }
        }
        _ => Some(in_state.clone()),
    }
}
