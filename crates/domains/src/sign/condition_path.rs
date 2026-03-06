use rustc_middle::mir::*;
use rustc_middle::ty::{Ty, TyCtxt, TyKind};

use super::abstract_value::{Sign, eq, ge, gt, le, lt, meet, neq};
use super::state::SignState;
use super::transfer::{eval_operand, sign_of_const};

fn switch_value_to_i128<'tcx>(tcx: TyCtxt<'tcx>, ty: Ty<'tcx>, value: u128) -> Option<i128> {
    let (bit_width, signed) = match ty.kind() {
        TyKind::Int(int_ty) => (
            int_ty
                .bit_width()
                .unwrap_or_else(|| tcx.data_layout.pointer_size.bits()),
            true,
        ),
        TyKind::Uint(uint_ty) => (
            uint_ty
                .bit_width()
                .unwrap_or_else(|| tcx.data_layout.pointer_size.bits()),
            false,
        ),
        TyKind::Bool => (1, false),
        TyKind::Char => (32, false),
        _ => return None,
    };
    Some(if signed {
        signed_bits_to_i128(value, bit_width)
    } else {
        unsigned_bits_to_i128(value, bit_width)
    })
}

fn unsigned_bits_to_i128(bits: u128, bit_width: u64) -> i128 {
    if bit_width == 128 {
        if bits <= i128::MAX as u128 {
            bits as i128
        } else {
            i128::MAX
        }
    } else {
        let mask = (1u128 << bit_width) - 1;
        (bits & mask) as i128
    }
}

fn signed_bits_to_i128(bits: u128, bit_width: u64) -> i128 {
    if bit_width == 128 {
        return bits as i128;
    }

    let sign_bit = 1u128 << (bit_width - 1);
    let mask = (1u128 << bit_width) - 1;
    let x = bits & mask;

    if (x & sign_bit) != 0 {
        (x as i128) - ((1u128 << bit_width) as i128)
    } else {
        x as i128
    }
}

fn sign_of_i128(v: i128) -> Sign {
    if v == 0 {
        Sign::Zero
    } else if v > 0 {
        Sign::Pos
    } else {
        Sign::Neg
    }
}

fn refine_place_with_sign<'tcx>(st: &mut SignState<'tcx>, place: Place<'tcx>, new_s: Sign) -> bool {
    let cur = st.get_sign(&place);
    let refined = meet(cur, new_s);
    if refined == Sign::Bot {
        return false;
    }
    st.set_sign(place, refined);
    true
}

fn find_last_cmp_assign<'tcx>(
    body: &'tcx Body<'tcx>,
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
                _ => None,
            };
        }
    }
    None
}

fn cmp_result(op: BinOp, l: Sign, r: Sign) -> Sign {
    match op {
        BinOp::Lt => lt(l, r),
        BinOp::Le => le(l, r),
        BinOp::Gt => gt(l, r),
        BinOp::Ge => ge(l, r),
        BinOp::Eq => eq(l, r),
        BinOp::Ne => neq(l, r),
        _ => Sign::Top,
    }
}

fn refine_cmp<'tcx>(
    tcx: TyCtxt<'tcx>,
    local_decls: &'tcx LocalDecls<'tcx>,
    st: &mut SignState<'tcx>,
    op: BinOp,
    truth: bool,
    left: &Operand<'tcx>,
    right: &Operand<'tcx>,
) -> Option<()> {
    let ls = eval_operand(tcx, local_decls, left, st);
    let rs = eval_operand(tcx, local_decls, right, st);
    let cmp = cmp_result(op, ls, rs);

    if truth && cmp == Sign::Zero {
        return None;
    }
    if !truth && cmp == Sign::Pos {
        return None;
    }

    match (op, truth) {
        (BinOp::Eq, true) | (BinOp::Ne, false) => {
            let inter = meet(ls, rs);
            if inter == Sign::Bot {
                return None;
            }
            if let Operand::Copy(p) | Operand::Move(p) = left {
                if !refine_place_with_sign(st, *p, inter) {
                    return None;
                }
            }
            if let Operand::Copy(p) | Operand::Move(p) = right {
                if !refine_place_with_sign(st, *p, inter) {
                    return None;
                }
            }
        }
        (BinOp::Lt, true) | (BinOp::Ge, false) => {
            if let Operand::Copy(p) | Operand::Move(p) = left {
                if rs == Sign::Zero && !refine_place_with_sign(st, *p, Sign::Neg) {
                    return None;
                }
            }
            if let Operand::Copy(p) | Operand::Move(p) = right {
                if ls == Sign::Zero && !refine_place_with_sign(st, *p, Sign::Pos) {
                    return None;
                }
            }
        }
        (BinOp::Gt, true) | (BinOp::Le, false) => {
            if let Operand::Copy(p) | Operand::Move(p) = left {
                if rs == Sign::Zero && !refine_place_with_sign(st, *p, Sign::Pos) {
                    return None;
                }
            }
            if let Operand::Copy(p) | Operand::Move(p) = right {
                if ls == Sign::Zero && !refine_place_with_sign(st, *p, Sign::Neg) {
                    return None;
                }
            }
        }
        _ => {}
    }

    Some(())
}

pub fn refine_edge<'tcx>(
    tcx: TyCtxt<'tcx>,
    body: &'tcx Body<'tcx>,
    pred: BasicBlock,
    succ: BasicBlock,
    in_state: &SignState<'tcx>,
) -> Option<SignState<'tcx>> {
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
                    let current = st.get_sign(place);
                    let mut succ_sign = Sign::Bot;
                    for &v in &values_for_succ {
                        if let Some(i) = switch_value_to_i128(tcx, discr_ty, v) {
                            succ_sign = super::abstract_value::join(succ_sign, sign_of_i128(i));
                        }
                    }

                    if succ_sign != Sign::Bot {
                        if !refine_place_with_sign(&mut st, *place, succ_sign) {
                            return None;
                        }
                    } else if is_otherwise && current != Sign::Top && current != Sign::Bot {
                        let mut all_sign = Sign::Bot;
                        for &v in &all_values {
                            if let Some(i) = switch_value_to_i128(tcx, discr_ty, v) {
                                all_sign = super::abstract_value::join(all_sign, sign_of_i128(i));
                            }
                        }
                        if meet(current, all_sign) == current {
                            return None;
                        }
                    }

                    if let TyKind::Bool = discr_ty.kind() {
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
                            if let Some((op, left, right)) =
                                find_last_cmp_assign(body, pred, *place)
                            {
                                refine_cmp(
                                    tcx,
                                    &body.local_decls,
                                    &mut st,
                                    op,
                                    truth,
                                    &left,
                                    &right,
                                )?;
                            }
                        }
                    }

                    Some(st)
                }
                Operand::Constant(c) => {
                    let c_sign = sign_of_const(c);
                    if c_sign == Sign::Bot || c_sign == Sign::Top {
                        return Some(st);
                    }

                    if let Some(si) = c.const_.try_to_scalar_int() {
                        let c_val = if matches!(c.ty().kind(), TyKind::Int(_)) {
                            signed_bits_to_i128(si.to_bits_unchecked(), si.size().bits())
                        } else {
                            unsigned_bits_to_i128(si.to_bits_unchecked(), si.size().bits())
                        };
                        let mut values_for_succ_i128: Vec<i128> = Vec::new();
                        for &v in &values_for_succ {
                            if let Some(i) = switch_value_to_i128(tcx, discr_ty, v) {
                                values_for_succ_i128.push(i);
                            }
                        }
                        let mut all_values_i128: Vec<i128> = Vec::new();
                        for &v in &all_values {
                            if let Some(i) = switch_value_to_i128(tcx, discr_ty, v) {
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
                    } else {
                        Some(st)
                    }
                }
            }
        }
        _ => Some(in_state.clone()),
    }
}
