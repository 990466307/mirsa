use rustc_middle::mir::*;
use rustc_middle::ty::{Ty, TyCtxt, TyKind};

use super::abstract_value::*;
use super::state::{InternvalState, bits_to_i128, scalar_layout};

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
    local_decls: &LocalDecls<'tcx>,
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
    local_decls: &LocalDecls<'tcx>,
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

fn slice_or_array_len<'tcx>(
    tcx: TyCtxt<'tcx>,
    local_decls: &LocalDecls<'tcx>,
    st: &InternvalState<'tcx>,
    place: Place<'tcx>,
) -> Option<Internval> {
    let ty = place.ty(local_decls, tcx).ty;
    match ty.kind() {
        TyKind::Array(_, len) => len
            .try_to_target_usize(tcx)
            .map(|len| Internval::new(len as i128, len as i128)),
        TyKind::Slice(_) => st.get_slice_meta(&place),
        TyKind::Ref(_, inner, _) => match inner.kind() {
            TyKind::Array(_, len) => len
                .try_to_target_usize(tcx)
                .map(|len| Internval::new(len as i128, len as i128)),
            TyKind::Slice(_) => st.get_slice_meta(&place),
            _ => None,
        },
        _ => None,
    }
}

fn operand_slice_or_array_len<'tcx>(
    tcx: TyCtxt<'tcx>,
    local_decls: &LocalDecls<'tcx>,
    st: &InternvalState<'tcx>,
    op: &Operand<'tcx>,
) -> Option<Internval> {
    match op {
        Operand::Copy(place) | Operand::Move(place) => {
            slice_or_array_len(tcx, local_decls, st, *place)
        }
        Operand::Constant(_) => None,
    }
}

fn operand_place<'tcx>(op: &Operand<'tcx>) -> Option<Place<'tcx>> {
    match op {
        Operand::Copy(place) | Operand::Move(place) => Some(*place),
        Operand::Constant(_) => None,
    }
}

fn singleton_nonnegative(iv: Internval) -> Option<u64> {
    if iv.is_empty() || iv.low != iv.high || iv.low < 0 {
        return None;
    }
    Some(iv.low as u64)
}

fn terminator_call_path<'tcx>(
    tcx: TyCtxt<'tcx>,
    local_decls: &LocalDecls<'tcx>,
    term: &Terminator<'tcx>,
) -> Option<String> {
    let TerminatorKind::Call { func, .. } = &term.kind else {
        return None;
    };
    let TyKind::FnDef(def_id, _) = func.ty(local_decls, tcx).kind() else {
        return None;
    };
    Some(tcx.def_path_str(*def_id))
}

// 当动态索引左值无法精确解析时，将其对应数组元素全部弱化为 Top。
fn eval_place<'tcx>(
    tcx: TyCtxt<'tcx>,
    local_decls: &LocalDecls<'tcx>,
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
    local_decls: &LocalDecls<'tcx>,
    op: &Operand<'tcx>,
    st: &InternvalState<'tcx>,
) -> Internval {
    match op {
        Operand::Copy(p) | Operand::Move(p) => eval_place(tcx, local_decls, *p, st),
        Operand::Constant(c) => internval_of_const(c),
    }
}

// 计算带溢出算术，并写入 (result, overflow) 元组区间。
fn eval_binary_op_with_overflow_internval<'tcx>(
    tcx: TyCtxt<'tcx>,
    st: &mut InternvalState<'tcx>,
    place: &Place<'tcx>,
    local_decls: &LocalDecls<'tcx>,
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
    local_decls: &LocalDecls<'tcx>,
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
        _ => Internval::top(),
    }
}

// 在区间域中计算一元运算。
fn eval_unary_op_internval<'tcx>(
    tcx: TyCtxt<'tcx>,
    st: &mut InternvalState<'tcx>,
    local_decls: &LocalDecls<'tcx>,
    op: &UnOp,
    arg: &Operand<'tcx>,
) -> Internval {
    let sa = eval_operand(tcx, local_decls, arg, st);

    match op {
        UnOp::Neg => neg(&sa),
        _ => Internval::top(),
    }
}

// 对单条 MIR 语句执行区间与等价关系的 transfer。
pub fn transfer_stmt<'tcx>(
    tcx: TyCtxt<'tcx>,
    st: &mut InternvalState<'tcx>,
    stmt: &Statement<'tcx>,
    local_decls: &LocalDecls<'tcx>,
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
                    return;
                }
                for p in targets {
                    st.set_internval(p, Internval::top());
                    st.clear_slice_meta(&p);
                    st.eq.kill(p);
                }
                return;
            }
            let dst_place = resolved_place.unwrap_or(*place);
            st.clear_slice_meta(&dst_place);
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

                Rvalue::UnaryOp(UnOp::PtrMetadata, op) => {
                    let len_iv = match op {
                        Operand::Copy(src) | Operand::Move(src) => {
                            st.get_slice_meta(src).unwrap_or_else(Internval::top)
                        }
                        Operand::Constant(_) => Internval::top(),
                    };
                    st.set_internval(dst_place, len_iv);
                }

                Rvalue::UnaryOp(op, arg) => {
                    let rhs_internval = eval_unary_op_internval(tcx, st, local_decls, op, arg);
                    st.set_internval(dst_place, rhs_internval);
                }

                Rvalue::Use(op) => {
                    let rhs_internval = eval_operand(tcx, local_decls, op, st);
                    st.set_internval(dst_place, rhs_internval);
                }

                Rvalue::Cast(cast_kind, op, dst_ty) => {
                    let rhs_internval = eval_cast_internval(tcx, st, local_decls, op, *dst_ty);
                    st.set_internval(dst_place, rhs_internval);
                    if matches!(cast_kind, CastKind::PointerCoercion(_, _)) {
                        if let Operand::Copy(src) | Operand::Move(src) = op {
                            let src_ty = src.ty(local_decls, tcx).ty;
                            if let TyKind::Ref(_, inner, _) = src_ty.kind() {
                                if let TyKind::Array(_, len) = inner.kind() {
                                    if let Some(len) = len.try_to_target_usize(tcx) {
                                        st.set_slice_meta(
                                            dst_place,
                                            Internval::new(len as i128, len as i128),
                                        );
                                        st.eq.union(dst_place, *src);
                                    }
                                }
                            }
                        }
                    }
                }

                Rvalue::Len(src) => {
                    let src_ty = src.ty(local_decls, tcx).ty;
                    let len_iv = match src_ty.kind() {
                        TyKind::Array(_, len) => len
                            .try_to_target_usize(tcx)
                            .map(|len| Internval::new(len as i128, len as i128))
                            .unwrap_or_else(Internval::top),
                        TyKind::Slice(_) => st.get_slice_meta(src).unwrap_or_else(Internval::top),
                        TyKind::Ref(_, inner, _) => match inner.kind() {
                            TyKind::Array(_, len) => len
                                .try_to_target_usize(tcx)
                                .map(|len| Internval::new(len as i128, len as i128))
                                .unwrap_or_else(Internval::top),
                            TyKind::Slice(_) => {
                                st.get_slice_meta(src).unwrap_or_else(Internval::top)
                            }
                            _ => Internval::top(),
                        },
                        _ => Internval::top(),
                    };
                    st.set_internval(dst_place, len_iv);
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
                            st.clear_slice_meta(&elem_place);
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
                            st.clear_slice_meta(&elem_place);
                        }
                    }
                    _ => st.set_internval(dst_place, Internval::top()),
                },

                Rvalue::Ref(_region, _borrow_kind, borrowed_place) => {
                    let borrowed_internval = eval_place(tcx, local_decls, *borrowed_place, st);
                    st.set_internval(dst_place, borrowed_internval);
                    let borrowed_ty = borrowed_place.ty(local_decls, tcx).ty;
                    match borrowed_ty.kind() {
                        TyKind::Array(_, len) => {
                            st.eq.union(dst_place, *borrowed_place);
                            if let Some(len) = len.try_to_target_usize(tcx) {
                                st.set_slice_meta(
                                    dst_place,
                                    Internval::new(len as i128, len as i128),
                                );
                            }
                        }
                        TyKind::Slice(_) => {
                            if let Some(len) = st.get_slice_meta(borrowed_place) {
                                st.set_slice_meta(dst_place, len);
                            }
                        }
                        _ => {}
                    }
                }

                _ => st.set_internval(dst_place, Internval::top()),
            }
        }
        _ => {}
    }
}

pub fn transfer_terminator<'tcx>(
    tcx: TyCtxt<'tcx>,
    st: &mut InternvalState<'tcx>,
    term: &Terminator<'tcx>,
    local_decls: &LocalDecls<'tcx>,
) {
    let TerminatorKind::Call {
        args, destination, ..
    } = &term.kind
    else {
        return;
    };
    let Some(path) = terminator_call_path(tcx, local_decls, term) else {
        return;
    };
    if !path.ends_with("::get_unchecked") && !path.ends_with("::get_unchecked_mut") {
        return;
    }
    if args.len() < 2 {
        return;
    }
    let Some(receiver_place) = operand_place(&args[0].node) else {
        return;
    };
    let index = eval_operand(tcx, local_decls, &args[1].node, st);
    let Some(index) = singleton_nonnegative(index) else {
        return;
    };
    let Some(len) = operand_slice_or_array_len(tcx, local_decls, st, &args[0].node)
        .and_then(singleton_nonnegative)
    else {
        return;
    };
    if index >= len {
        return;
    }
    let mut source_place = receiver_place;
    let mut source_is_ref = true;
    for candidate in st.internval.keys().copied() {
        if !st.eq.equiv_readonly(receiver_place, candidate) {
            continue;
        }
        if matches!(
            candidate.ty(local_decls, tcx).ty.kind(),
            TyKind::Array(_, _)
        ) {
            source_place = candidate;
            source_is_ref = false;
            break;
        }
    }
    if source_is_ref {
        for candidate in st.internval.keys().copied() {
            if !st.eq.equiv_readonly(receiver_place, candidate) {
                continue;
            }
            if let TyKind::Ref(_, inner, _) = candidate.ty(local_decls, tcx).ty.kind() {
                if matches!(inner.kind(), TyKind::Array(_, _)) {
                    source_place = candidate;
                    break;
                }
            }
        }
    }
    let elem_place = if source_is_ref {
        source_place.project_deeper(
            &[
                ProjectionElem::Deref,
                ProjectionElem::ConstantIndex {
                    offset: index,
                    min_length: len,
                    from_end: false,
                },
            ],
            tcx,
        )
    } else {
        source_place.project_deeper(
            &[ProjectionElem::ConstantIndex {
                offset: index,
                min_length: len,
                from_end: false,
            }],
            tcx,
        )
    };
    let dst_deref = destination.project_deeper(&[ProjectionElem::Deref], tcx);
    let elem_iv = eval_place(tcx, local_decls, elem_place, st);
    st.set_internval(dst_deref, elem_iv);
}
