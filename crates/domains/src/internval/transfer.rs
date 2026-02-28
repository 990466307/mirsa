use rustc_middle::mir::*;
use rustc_middle::ty::{Ty, TyCtxt, TyKind};

use super::abstract_value::*;
use super::state::InternvalState;

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

fn bits_to_i128(bits: u128, bit_width: u64, signed: bool) -> i128 {
    if signed {
        signed_bits_to_i128(bits, bit_width)
    } else {
        unsigned_bits_to_i128(bits, bit_width)
    }
}

fn i128_to_bits(value: i128, bit_width: u64) -> u128 {
    if bit_width == 128 {
        value as u128
    } else {
        let mask = (1u128 << bit_width) - 1;
        (value as u128) & mask
    }
}

// 在区间域中计算 Cast；无法可靠保持顺序时保守回退为 Top。
fn eval_cast_internval<'tcx>(
    tcx: TyCtxt<'tcx>,
    st: &InternvalState<'tcx>,
    local_decls: &'tcx LocalDecls<'tcx>,
    op: &Operand<'tcx>,
    dst_ty: Ty<'tcx>,
) -> Internval {
    let src_ty = op.ty(local_decls, tcx);
    let src_iv = eval_operand(tcx, local_decls, op, st);
    if src_iv.is_empty() {
        return Internval::empty();
    }

    let Some((src_bw, src_signed)) = scalar_layout(tcx, src_ty) else {
        return Internval::top();
    };
    let Some((dst_bw, dst_signed)) = scalar_layout(tcx, dst_ty) else {
        return Internval::top();
    };

    if src_iv.low == src_iv.high {
        let casted = (|| -> Option<i128> {
            let (src_bw, _src_signed) = scalar_layout(tcx, src_ty)?;
            let (dst_bw, dst_signed) = scalar_layout(tcx, dst_ty)?;
            let bits = i128_to_bits(src_iv.low, src_bw);
            Some(bits_to_i128(bits, dst_bw, dst_signed))
        })();
        return casted
            .map(|v| Internval::new(v, v))
            .unwrap_or(Internval::top());
    }

    if src_signed == dst_signed && dst_bw >= src_bw {
        let (src_min, src_max) = if src_signed {
            if src_bw == 128 {
                (i128::MIN, i128::MAX)
            } else {
                let max = (1i128 << (src_bw - 1)) - 1;
                let min = -(1i128 << (src_bw - 1));
                (min, max)
            }
        } else if src_bw == 128 {
            (0, i128::MAX)
        } else {
            (0, ((1u128 << src_bw) - 1) as i128)
        };
        let clipped = intersect(&src_iv, &Internval::new(src_min, src_max));
        if clipped.is_empty() {
            return Internval::empty();
        }
        return clipped;
    }

    Internval::top()
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

// 将 MIR 常量操作数求值为区间值。
pub(crate) fn internval_of_const<'tcx>(c: &ConstOperand<'tcx>) -> Internval {
    let ty = c.ty();
    let signed = match ty.kind() {
        TyKind::Int(_) => true,
        TyKind::Uint(_) | TyKind::Bool | TyKind::Char => false,
        _ => return Internval::top(),
    };
    let bit_width_from_ty = match ty.kind() {
        TyKind::Bool => 1,
        TyKind::Char => 32,
        _ => 0,
    };
    let k = c.const_;
    if let Some(si) = k.try_to_scalar_int() {
        let bit_width = match ty.kind() {
            TyKind::Int(_) | TyKind::Uint(_) => si.size().bits(),
            _ => bit_width_from_ty,
        };
        let v = bits_to_i128(si.to_bits_unchecked(), bit_width, signed);
        Internval::new(v, v)
    } else {
        Internval::top()
    }
}

// 判断 place 是否包含运行时索引投影。
fn has_runtime_index<'tcx>(place: Place<'tcx>) -> bool {
    place
        .projection
        .iter()
        .any(|elem| matches!(elem, ProjectionElem::Index(_)))
}

// 当索引区间为单点时，把动态索引解析成常量索引。
fn resolve_indexed_place<'tcx>(
    tcx: TyCtxt<'tcx>,
    local_decls: &'tcx LocalDecls<'tcx>,
    st: &InternvalState<'tcx>,
    place: Place<'tcx>,
) -> Option<Place<'tcx>> {
    if !has_runtime_index(place) {
        return Some(place);
    }

    let mut resolved = Place::from(place.local);
    for elem in place.projection.iter() {
        match elem {
            ProjectionElem::Index(local) => {
                let idx_iv = st.get_internval(&Place::from(local));
                let arr_ty = resolved.ty(local_decls, tcx).ty;
                let len = match arr_ty.kind() {
                    TyKind::Array(_, len) => len.try_to_target_usize(tcx)? as u64,
                    _ => return None,
                };
                if !idx_iv.is_empty() {
                    let max_idx = len.saturating_sub(1) as i128;
                    if idx_iv.low < 0 || idx_iv.high > max_idx {
                        println!(
                            "Warning: potential array out-of-bounds access, index {:?}, valid range [0, {}]",
                            idx_iv, max_idx
                        );
                    }
                }
                if idx_iv.is_empty() || idx_iv.low != idx_iv.high || idx_iv.low < 0 {
                    return None;
                }
                let idx = idx_iv.low as u64;
                if idx >= len {
                    return None;
                }
                resolved = resolved.project_deeper(
                    &[ProjectionElem::ConstantIndex {
                        offset: idx,
                        min_length: len,
                        from_end: false,
                    }],
                    tcx,
                );
            }
            _ => {
                resolved = resolved.project_deeper(&[elem], tcx);
            }
        }
    }
    Some(resolved)
}

// 当动态索引左值无法精确解析时，将其对应数组元素全部弱化为 Top。
fn eval_place<'tcx>(
    tcx: TyCtxt<'tcx>,
    local_decls: &'tcx LocalDecls<'tcx>,
    place: Place<'tcx>,
    st: &InternvalState<'tcx>,
) -> Internval {
    if let Some(resolved) = resolve_indexed_place(tcx, local_decls, st, place) {
        st.get_internval(&resolved)
    } else if has_runtime_index(place) {
        Internval::top()
    } else {
        st.get_internval(&place)
    }
}

// 将操作数求值为区间值。
pub(crate) fn eval_operand<'tcx>(
    tcx: TyCtxt<'tcx>,
    local_decls: &'tcx LocalDecls<'tcx>,
    op: &Operand<'tcx>,
    st: &InternvalState<'tcx>,
) -> Internval {
    match op {
        Operand::Copy(p) | Operand::Move(p) => eval_place(tcx, local_decls, *p, st),
        Operand::Constant(c) => internval_of_const(c),
    }
}

// 对单条 MIR 语句执行区间与等价关系的 transfer。
pub fn transfer_stmt<'tcx>(
    tcx: TyCtxt<'tcx>,
    st: &mut InternvalState<'tcx>,
    stmt: &Statement<'tcx>,
    local_decls: &'tcx LocalDecls<'tcx>,
) {
    let kind = &stmt.kind;
    match kind {
        StatementKind::Assign(assign) => {
            let (place, rvalue) = &**assign;
            let resolved_place = resolve_indexed_place(tcx, local_decls, st, *place);
            if resolved_place.is_none() {
                let targets: Vec<Place<'tcx>> = st
                    .internval
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
                    st.set_internval(p, Internval::top());
                    st.eq.kill(p);
                }
                return;
            }
            let dst_place = resolved_place.unwrap_or(*place);
            // 关系型
            match rvalue {
                Rvalue::Use(op) => match op {
                    Operand::Copy(src) | Operand::Move(src) => {
                        st.eq.kill(dst_place);
                        let resolved_src =
                            resolve_indexed_place(tcx, local_decls, st, *src).unwrap_or(*src);
                        st.eq.union(dst_place, resolved_src);
                    }
                    Operand::Constant(_) => {
                        st.eq.kill(dst_place);
                    }
                },
                _ => {
                    st.eq.kill(dst_place);
                }
            }
            match rvalue {
                Rvalue::BinaryOp(op, ops) => match op {
                    BinOp::AddWithOverflow | BinOp::SubWithOverflow | BinOp::MulWithOverflow => {
                        eval_binary_op_with_overflow_internval(
                            tcx,
                            st,
                            &dst_place,
                            local_decls,
                            op,
                            ops,
                        );
                    }
                    _ => {
                        let rhs_internval = eval_binary_op_internval(tcx, st, local_decls, op, ops);
                        st.set_internval(dst_place, rhs_internval);
                    }
                },

                Rvalue::UnaryOp(op, arg) => {
                    let rhs_internval = eval_unary_op_internval(tcx, st, local_decls, op, arg);
                    st.set_internval(dst_place, rhs_internval);
                }

                Rvalue::Use(op) => {
                    let rhs_internval = eval_operand(tcx, local_decls, op, st);
                    st.set_internval(dst_place, rhs_internval);
                }

                Rvalue::Cast(_cast_kind, op, dst_ty) => {
                    let rhs_internval = eval_cast_internval(tcx, st, local_decls, op, *dst_ty);
                    st.set_internval(dst_place, rhs_internval);
                }

                Rvalue::Aggregate(kind, indexvec) => match kind.as_ref() {
                    AggregateKind::Tuple => {
                        for (i, op) in indexvec.iter().enumerate() {
                            let elem_place = dst_place.project_deeper(
                                &[ProjectionElem::Field(i.into(), op.ty(local_decls, tcx))],
                                tcx,
                            );
                            let elem_internval = eval_operand(tcx, local_decls, op, st);
                            st.set_internval(elem_place, elem_internval);
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
                            let elem_internval = eval_operand(tcx, local_decls, op, st);
                            st.set_internval(elem_place, elem_internval);
                        }
                    }
                    _ => {
                        println!(
                            "Not Support: unhandled Aggregate kind in internval analysis: {:?}",
                            kind
                        );
                        st.set_internval(dst_place, Internval::top());
                    }
                },

                Rvalue::Ref(_region, _borrow_kind, borrowed_place) => {
                    let borrowed_internval = eval_place(tcx, local_decls, *borrowed_place, st);
                    st.set_internval(dst_place, borrowed_internval);
                }

                _ => {
                    println!(
                        "Not Support: unhandled Rvalue in internval analysis: {:?}",
                        rvalue
                    );
                    st.set_internval(dst_place, Internval::top());
                }
            }
        }
        _ => {
            println!(
                "Not Support: unhandled Statement in internval analysis: {:?}",
                kind
            );
        }
    }
}

// 计算带溢出算术，并写入 (result, overflow) 元组区间。
fn eval_binary_op_with_overflow_internval<'tcx>(
    tcx: TyCtxt<'tcx>,
    st: &mut InternvalState<'tcx>,
    place: &Place<'tcx>,
    local_decls: &'tcx LocalDecls<'tcx>,
    op: &BinOp,
    ops: &Box<(Operand<'tcx>, Operand<'tcx>)>,
) {
    let (a, b) = &**ops;
    let sa = eval_operand(tcx, local_decls, a, st);
    let sb = eval_operand(tcx, local_decls, b, st);

    let result_sign = match op {
        BinOp::AddWithOverflow => add(&sa, &sb),
        BinOp::SubWithOverflow => sub(&sa, &sb),
        BinOp::MulWithOverflow => mul(&sa, &sb),
        _ => unreachable!(),
    };

    let operand_ty = match a {
        Operand::Copy(place) | Operand::Move(place) => place.ty(local_decls, tcx).ty,
        Operand::Constant(const_) => const_.ty(),
    };

    let result_place = place.project_deeper(&[ProjectionElem::Field(0u32.into(), operand_ty)], tcx);
    st.set_internval(result_place, result_sign);

    let overflow_place =
        place.project_deeper(&[ProjectionElem::Field(1u32.into(), tcx.types.bool)], tcx);
    st.set_internval(overflow_place, Internval::new(0, 0));
}

// 在区间域中计算二元运算。
fn eval_binary_op_internval<'tcx>(
    tcx: TyCtxt<'tcx>,
    st: &mut InternvalState<'tcx>,
    local_decls: &'tcx LocalDecls<'tcx>,
    op: &BinOp,
    ops: &Box<(Operand<'tcx>, Operand<'tcx>)>,
) -> Internval {
    let (a, b) = &**ops;
    let sa = eval_operand(tcx, local_decls, a, st);
    let sb = eval_operand(tcx, local_decls, b, st);

    match op {
        BinOp::Add => add(&sa, &sb),
        BinOp::Sub => sub(&sa, &sb),
        BinOp::Mul => mul(&sa, &sb),
        BinOp::Div => div(&sa, &sb),
        BinOp::BitAnd => bitand(&sa, &sb),
        BinOp::BitOr => bitor(&sa, &sb),
        BinOp::BitXor => bitxor(&sa, &sb),
        BinOp::Le => le(&sa, &sb),
        BinOp::Lt => lt(&sa, &sb),
        BinOp::Ge => ge(&sa, &sb),
        BinOp::Gt => gt(&sa, &sb),
        BinOp::Eq => eq(&sa, &sb),
        BinOp::Ne => neq(&sa, &sb),
        _ => {
            println!(
                "Not Support: unhandled binary op in internval analysis: {:?}",
                op
            );
            Internval::top()
        }
    }
}

// 在区间域中计算一元运算。
fn eval_unary_op_internval<'tcx>(
    tcx: TyCtxt<'tcx>,
    st: &mut InternvalState<'tcx>,
    local_decls: &'tcx LocalDecls<'tcx>,
    op: &UnOp,
    arg: &Operand<'tcx>,
) -> Internval {
    let sa = eval_operand(tcx, local_decls, arg, st);

    match op {
        UnOp::Neg => neg(&sa),
        _ => {
            println!(
                "Not Support: unhandled unary op in internval analysis: {:?}",
                op
            );
            Internval::top()
        }
    }
}
