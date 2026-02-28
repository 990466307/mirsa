use rustc_middle::mir::*;
use rustc_middle::ty::{Ty, TyCtxt, TyKind};

use super::abstract_value::*;
use super::state::SignState;

fn signed_to_i128(bits: u128, bw: u64) -> i128 {
    if bw == 128 {
        return bits as i128;
    }

    let sign_bit = 1u128 << (bw - 1);
    let mask = (1u128 << bw) - 1;
    let x = bits & mask;

    if (x & sign_bit) != 0 {
        (x as i128) - ((1u128 << bw) as i128)
    } else {
        x as i128
    }
}

pub(crate) fn sign_of_const<'tcx>(c: &ConstOperand<'tcx>) -> Sign {
    let ty = c.ty();

    let is_signed = match ty.kind() {
        TyKind::Int(_) => true,
        TyKind::Uint(_) => false,
        _ => return Sign::Top, // 不是整数：不在本域内
    };
    let k = c.const_;
    if let Some(si) = k.try_to_scalar_int() {
        let bw = si.size().bits();
        let bits = si.to_bits_unchecked();
        if is_signed {
            let v: i128 = signed_to_i128(bits, bw);
            if v == 0 {
                Sign::Zero
            } else if v > 0 {
                Sign::Pos
            } else {
                Sign::Neg
            }
        } else {
            if bits == 0 { Sign::Zero } else { Sign::Pos }
        }
    } else {
        Sign::Top
    }
}

fn scalar_layout<'tcx>(tcx: TyCtxt<'tcx>, ty: Ty<'tcx>) -> Option<(u64, bool)> {
    match ty.kind() {
        TyKind::Int(int_ty) => Some((
            int_ty
                .bit_width()
                .unwrap_or_else(|| tcx.data_layout.pointer_size.bits()),
            true,
        )),
        TyKind::Uint(uint_ty) => Some((
            uint_ty
                .bit_width()
                .unwrap_or_else(|| tcx.data_layout.pointer_size.bits()),
            false,
        )),
        TyKind::Bool => Some((1, false)),
        TyKind::Char => Some((32, false)),
        _ => None,
    }
}

fn has_runtime_index<'tcx>(place: Place<'tcx>) -> bool {
    place
        .projection
        .iter()
        .any(|elem| matches!(elem, ProjectionElem::Index(_)))
}

fn resolve_indexed_place<'tcx>(
    tcx: TyCtxt<'tcx>,
    local_decls: &'tcx LocalDecls<'tcx>,
    st: &SignState<'tcx>,
    place: Place<'tcx>,
) -> Option<Place<'tcx>> {
    if !has_runtime_index(place) {
        return Some(place);
    }

    let mut resolved = Place::from(place.local);
    for elem in place.projection.iter() {
        match elem {
            ProjectionElem::Index(local) => {
                let idx_sign = st.get_sign(&Place::from(local));
                let arr_ty = resolved.ty(local_decls, tcx).ty;
                let len = match arr_ty.kind() {
                    TyKind::Array(_, len) => len.try_to_target_usize(tcx)? as u64,
                    _ => return None,
                };

                match idx_sign {
                    Sign::Zero => {
                        if len == 0 {
                            return None;
                        }
                        resolved = resolved.project_deeper(
                            &[ProjectionElem::ConstantIndex {
                                offset: 0,
                                min_length: len,
                                from_end: false,
                            }],
                            tcx,
                        );
                    }
                    Sign::Neg => {
                        println!(
                            "Warning: potential array out-of-bounds access, index sign {:?}, valid range [0, {}]",
                            idx_sign,
                            len.saturating_sub(1)
                        );
                        return None;
                    }
                    _ => return None,
                }
            }
            _ => {
                resolved = resolved.project_deeper(&[elem], tcx);
            }
        }
    }
    Some(resolved)
}

fn eval_place<'tcx>(
    tcx: TyCtxt<'tcx>,
    local_decls: &'tcx LocalDecls<'tcx>,
    place: Place<'tcx>,
    st: &SignState<'tcx>,
) -> Sign {
    if let Some(resolved) = resolve_indexed_place(tcx, local_decls, st, place) {
        st.get_sign(&resolved)
    } else if has_runtime_index(place) {
        Sign::Top
    } else {
        st.get_sign(&place)
    }
}

pub(crate) fn eval_operand<'tcx>(
    tcx: TyCtxt<'tcx>,
    local_decls: &'tcx LocalDecls<'tcx>,
    op: &Operand<'tcx>,
    st: &SignState<'tcx>,
) -> Sign {
    match op {
        Operand::Copy(p) | Operand::Move(p) => eval_place(tcx, local_decls, *p, st),
        Operand::Constant(c) => sign_of_const(c),
    }
}

fn eval_cast_sign<'tcx>(
    tcx: TyCtxt<'tcx>,
    st: &SignState<'tcx>,
    local_decls: &'tcx LocalDecls<'tcx>,
    op: &Operand<'tcx>,
    dst_ty: Ty<'tcx>,
) -> Sign {
    let src_ty = op.ty(local_decls, tcx);
    let src_sign = eval_operand(tcx, local_decls, op, st);
    if src_sign == Sign::Bot {
        return Sign::Bot;
    }

    if scalar_layout(tcx, src_ty).is_none() || scalar_layout(tcx, dst_ty).is_none() {
        return Sign::Top;
    }

    let (_, dst_signed) = scalar_layout(tcx, dst_ty).unwrap();
    match src_sign {
        Sign::Zero => Sign::Zero,
        Sign::Pos => Sign::Pos,
        Sign::Neg => {
            if dst_signed {
                Sign::Neg
            } else {
                Sign::Top
            }
        }
        Sign::Top => Sign::Top,
        Sign::Bot => Sign::Bot,
    }
}

pub fn transfer_stmt<'tcx>(
    tcx: TyCtxt<'tcx>,
    st: &mut SignState<'tcx>,
    stmt: &Statement<'tcx>,
    local_decls: &'tcx LocalDecls<'tcx>,
) {
    let StatementKind::Assign(assign) = &stmt.kind else {
        println!(
            "Warning: unhandled Statement in sign analysis: {:?}",
            stmt.kind
        );
        return;
    };
    let (place, rvalue) = &**assign;
    let resolved_place = resolve_indexed_place(tcx, local_decls, st, *place);
    if resolved_place.is_none() {
        let targets: Vec<Place<'tcx>> = st
            .locals
            .keys()
            .copied()
            .filter(|candidate| {
                if place.local != candidate.local {
                    return false;
                }
                if place.projection.len() != candidate.projection.len() {
                    return false;
                }
                place
                    .projection
                    .iter()
                    .zip(candidate.projection.iter())
                    .all(|(l, r)| match l {
                        ProjectionElem::Index(_) => {
                            matches!(r, ProjectionElem::ConstantIndex { .. })
                        }
                        _ => l == r,
                    })
            })
            .collect();
        if targets.is_empty() {
            println!(
                "Warning: unresolved indexed lhs {:?}, but no concrete array elements found.",
                place
            );
            return;
        }
        for p in targets {
            st.set_sign(p, Sign::Top);
        }
        return;
    }
    let dst_place = resolved_place.unwrap_or(*place);

    match rvalue {
        Rvalue::BinaryOp(op, ops) => match op {
            BinOp::AddWithOverflow | BinOp::SubWithOverflow | BinOp::MulWithOverflow => {
                eval_binary_op_with_overflow_sign(tcx, st, &dst_place, local_decls, op, ops);
            }
            _ => {
                let rhs_sign = eval_binary_op_sign(tcx, st, local_decls, op, ops);
                st.set_sign(dst_place, rhs_sign);
            }
        },

        Rvalue::UnaryOp(op, arg) => {
            let rhs_sign = eval_unary_op_sign(tcx, st, local_decls, op, arg);
            st.set_sign(dst_place, rhs_sign);
        }

        Rvalue::Use(op) => {
            let rhs_sign = eval_operand(tcx, local_decls, op, st);
            st.set_sign(dst_place, rhs_sign);
        }

        Rvalue::Cast(_cast_kind, op, dst_ty) => {
            let rhs_sign = eval_cast_sign(tcx, st, local_decls, op, *dst_ty);
            st.set_sign(dst_place, rhs_sign);
        }

        Rvalue::Aggregate(kind, indexvec) => {
            match kind.as_ref() {
                AggregateKind::Tuple => {
                    for (i, op) in indexvec.iter().enumerate() {
                        let elem_place = dst_place.project_deeper(
                            &[ProjectionElem::Field(i.into(), op.ty(local_decls, tcx))],
                            tcx,
                        );
                        let elem_sign = eval_operand(tcx, local_decls, op, st);
                        st.set_sign(elem_place, elem_sign);
                    }
                }
                AggregateKind::Array(_elem_ty) => {
                    let len = indexvec.len() as u64;
                    for (i, op) in indexvec.iter().enumerate() {
                        let elem_place = dst_place.project_deeper(
                            &[ProjectionElem::ConstantIndex {
                                offset: i as u64,
                                min_length: len,
                                from_end: false,
                            }],
                            tcx,
                        );
                        let elem_sign = eval_operand(tcx, local_decls, op, st);
                        st.set_sign(elem_place, elem_sign);
                    }
                }
                _ => {
                    println!(
                        "Warning: unhandled Aggregate kind in sign analysis: {:?}",
                        kind
                    );
                    st.set_sign(dst_place, Sign::Top);
                }
            }
        }

        Rvalue::Ref(_region, _borrow_kind, borrowed_place) => {
            let borrowed_sign = eval_place(tcx, local_decls, *borrowed_place, st);
            println!(
                "{:?} is a reference to {:?} with sign {:?}",
                dst_place, borrowed_place, borrowed_sign
            );
            st.set_sign(dst_place, borrowed_sign);
        }

        _ => {
            println!("Warning: unhandled Rvalue in sign analysis: {:?}", rvalue);
            st.set_sign(dst_place, Sign::Top);
        }
    }
}

fn eval_binary_op_with_overflow_sign<'tcx>(
    tcx: TyCtxt<'tcx>,
    st: &mut SignState<'tcx>,
    place: &Place<'tcx>,
    local_decls: &'tcx LocalDecls<'tcx>,
    op: &BinOp,
    ops: &Box<(Operand<'tcx>, Operand<'tcx>)>,
) {
    let (a, b) = &**ops;
    let sa = eval_operand(tcx, local_decls, a, st);
    let sb = eval_operand(tcx, local_decls, b, st);

    let result_sign = match op {
        BinOp::AddWithOverflow => add(sa, sb),
        BinOp::SubWithOverflow => sub(sa, sb),
        BinOp::MulWithOverflow => mul(sa, sb),
        _ => unreachable!(),
    };

    let operand_ty = match a {
        Operand::Copy(place) | Operand::Move(place) => place.ty(local_decls, tcx).ty,
        Operand::Constant(const_) => const_.ty(),
    };

    let result_place = place.project_deeper(&[ProjectionElem::Field(0u32.into(), operand_ty)], tcx);
    st.set_sign(result_place, result_sign);

    let overflow_place =
        place.project_deeper(&[ProjectionElem::Field(1u32.into(), tcx.types.bool)], tcx);
    st.set_sign(overflow_place, Sign::Top);
}

fn eval_binary_op_sign<'tcx>(
    tcx: TyCtxt<'tcx>,
    st: &mut SignState<'tcx>,
    local_decls: &'tcx LocalDecls<'tcx>,
    op: &BinOp,
    ops: &Box<(Operand<'tcx>, Operand<'tcx>)>,
) -> Sign {
    let (a, b) = &**ops;
    let sa = eval_operand(tcx, local_decls, a, st);
    let sb = eval_operand(tcx, local_decls, b, st);

    use rustc_middle::mir::BinOp::*;
    match op {
        Add | AddUnchecked => add(sa, sb),
        Sub | SubUnchecked => sub(sa, sb),
        Mul | MulUnchecked => mul(sa, sb),
        Div => div(sa, sb),
        Lt => lt(sa, sb),
        Le => le(sa, sb),
        Gt => gt(sa, sb),
        Ge => ge(sa, sb),
        Eq => eq(sa, sb),
        Ne => neq(sa, sb),
        _ => Sign::Top,
    }
}

fn eval_unary_op_sign<'tcx>(
    tcx: TyCtxt<'tcx>,
    st: &mut SignState<'tcx>,
    local_decls: &'tcx LocalDecls<'tcx>,
    op: &UnOp,
    arg: &Operand<'tcx>,
) -> Sign {
    let s = eval_operand(tcx, local_decls, arg, st);
    match op {
        UnOp::Neg => neg(s),
        _ => Sign::Top,
    }
}
