use rustc_middle::mir::*;
use rustc_middle::ty::{Ty, TyCtxt, TyKind};

use crate::framework::symbolic::SymbolicState;

use super::abstract_value::*;
use super::state::IntervalState;

fn i128_to_bits(value: i128, bit_width: u64) -> u128 {
    if bit_width == 128 {
        value as u128
    } else {
        let mask = (1u128 << bit_width) - 1;
        (value as u128) & mask
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

fn bits_to_i128(bits: u128, bit_width: u64, signed: bool) -> i128 {
    if signed {
        signed_bits_to_i128(bits, bit_width)
    } else {
        unsigned_bits_to_i128(bits, bit_width)
    }
}

pub(crate) fn switch_value_to_i128<'tcx>(
    tcx: TyCtxt<'tcx>,
    ty: Ty<'tcx>,
    value: u128,
) -> Option<i128> {
    let (bit_width, signed) = scalar_layout(tcx, ty)?;
    Some(bits_to_i128(value, bit_width, signed))
}

// 在区间域中计算 Cast；无法可靠保持顺序时保守回退为 Top。
fn eval_cast_interval<'tcx>(
    tcx: TyCtxt<'tcx>,
    st: &IntervalState<'tcx>,
    local_decls: &LocalDecls<'tcx>,
    op: &Operand<'tcx>,
    dst_ty: Ty<'tcx>,
) -> Interval {
    let src_ty = op.ty(local_decls, tcx);
    let src_iv = eval_operand(tcx, local_decls, op, st);
    if src_iv.is_empty() {
        return Interval::empty();
    }

    let Some((src_bw, src_signed)) = scalar_layout(tcx, src_ty) else {
        return Interval::top();
    };
    let Some((dst_bw, dst_signed)) = scalar_layout(tcx, dst_ty) else {
        return Interval::top();
    };

    if src_iv.low == src_iv.high {
        let casted = (|| -> Option<i128> {
            let (src_bw, _src_signed) = scalar_layout(tcx, src_ty)?;
            let (dst_bw, dst_signed) = scalar_layout(tcx, dst_ty)?;
            let bits = i128_to_bits(src_iv.low, src_bw);
            Some(bits_to_i128(bits, dst_bw, dst_signed))
        })();
        return casted
            .map(|v| Interval::new(v, v))
            .unwrap_or(Interval::top());
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
        let clipped = intersect(&src_iv, &Interval::new(src_min, src_max));
        if clipped.is_empty() {
            return Interval::empty();
        }
        return clipped;
    }

    Interval::top()
}

// 将 MIR 常量操作数求值为区间值。
pub(crate) fn interval_of_const<'tcx>(c: &ConstOperand<'tcx>) -> Interval {
    let ty = c.ty();
    let signed = match ty.kind() {
        TyKind::Int(_) => true,
        TyKind::Uint(_) | TyKind::Bool | TyKind::Char => false,
        _ => return Interval::top(),
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
        Interval::new(v, v)
    } else {
        Interval::top()
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
    st: &IntervalState<'tcx>,
    place: Place<'tcx>,
) -> Option<Place<'tcx>> {
    if !has_runtime_index(place) {
        return Some(place);
    }

    let mut resolved = Place::from(place.local);
    for elem in place.projection.iter() {
        match elem {
            ProjectionElem::Index(local) => {
                let idx_iv = st.get_interval(&Place::from(local));
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

fn operand_place<'tcx>(op: &Operand<'tcx>) -> Option<Place<'tcx>> {
    match op {
        Operand::Copy(place) | Operand::Move(place) => Some(*place),
        Operand::Constant(_) => None,
    }
}

fn singleton_nonnegative(iv: Interval) -> Option<u64> {
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

fn static_len<'tcx>(tcx: TyCtxt<'tcx>, ty: Ty<'tcx>) -> Option<Interval> {
    match ty.kind() {
        TyKind::Array(_, len) => len
            .try_to_target_usize(tcx)
            .map(|len| Interval::new(len as i128, len as i128)),
        TyKind::Ref(_, inner, _) => static_len(tcx, *inner),
        _ => None,
    }
}

fn place_len<'tcx>(
    tcx: TyCtxt<'tcx>,
    local_decls: &LocalDecls<'tcx>,
    st: &IntervalState<'tcx>,
    place: Place<'tcx>,
) -> Interval {
    let ty = place.ty(local_decls, tcx).ty;
    if let Some(len) = st.get_len(&place) {
        return len;
    }
    match ty.kind() {
        TyKind::Slice(_) => Interval::top(),
        TyKind::Ref(_, inner, _) if matches!(inner.kind(), TyKind::Slice(_)) => Interval::top(),
        _ => static_len(tcx, ty).unwrap_or_else(Interval::top),
    }
}

pub(crate) fn operand_len<'tcx>(
    tcx: TyCtxt<'tcx>,
    local_decls: &LocalDecls<'tcx>,
    st: &IntervalState<'tcx>,
    op: &Operand<'tcx>,
) -> Interval {
    match op {
        Operand::Copy(place) | Operand::Move(place) => place_len(tcx, local_decls, st, *place),
        Operand::Constant(_) => Interval::top(),
    }
}

pub(crate) fn operand_known_len<'tcx>(
    st: &IntervalState<'tcx>,
    op: &Operand<'tcx>,
) -> Option<Interval> {
    match op {
        Operand::Copy(place) | Operand::Move(place) => st.get_len(place),
        Operand::Constant(_) => None,
    }
}

fn slice_or_array_len<'tcx>(
    tcx: TyCtxt<'tcx>,
    local_decls: &LocalDecls<'tcx>,
    st: &IntervalState<'tcx>,
    place: Place<'tcx>,
) -> Option<Interval> {
    let ty = place.ty(local_decls, tcx).ty;
    match ty.kind() {
        TyKind::Array(_, len) => len
            .try_to_target_usize(tcx)
            .map(|len| Interval::new(len as i128, len as i128)),
        TyKind::Slice(_) => st.get_len(&place),
        TyKind::Ref(_, inner, _) => match inner.kind() {
            TyKind::Array(_, len) => len
                .try_to_target_usize(tcx)
                .map(|len| Interval::new(len as i128, len as i128)),
            TyKind::Slice(_) => st.get_len(&place),
            _ => None,
        },
        _ => None,
    }
}

fn operand_slice_or_array_len<'tcx>(
    tcx: TyCtxt<'tcx>,
    local_decls: &LocalDecls<'tcx>,
    st: &IntervalState<'tcx>,
    op: &Operand<'tcx>,
) -> Option<Interval> {
    match op {
        Operand::Copy(place) | Operand::Move(place) => {
            slice_or_array_len(tcx, local_decls, st, *place)
        }
        Operand::Constant(_) => None,
    }
}

fn raw_ptr_pointee_len<'tcx>(tcx: TyCtxt<'tcx>, ty: Ty<'tcx>) -> Option<Interval> {
    let TyKind::RawPtr(inner, _) = ty.kind() else {
        return None;
    };
    static_len(tcx, *inner)
}

fn place_object_len<'tcx>(
    tcx: TyCtxt<'tcx>,
    local_decls: &LocalDecls<'tcx>,
    st: &IntervalState<'tcx>,
    place: Place<'tcx>,
) -> Option<Interval> {
    st.get_len(&place)
        .or_else(|| static_len(tcx, place.ty(local_decls, tcx).ty))
}

fn operand_object_len<'tcx>(
    tcx: TyCtxt<'tcx>,
    local_decls: &LocalDecls<'tcx>,
    st: &IntervalState<'tcx>,
    op: &Operand<'tcx>,
) -> Option<Interval> {
    match op {
        Operand::Copy(place) | Operand::Move(place) => {
            place_object_len(tcx, local_decls, st, *place)
        }
        Operand::Constant(_) => None,
    }
}

fn rvalue_len<'tcx>(
    tcx: TyCtxt<'tcx>,
    local_decls: &LocalDecls<'tcx>,
    st: &IntervalState<'tcx>,
    rvalue: &Rvalue<'tcx>,
) -> Option<Interval> {
    match rvalue {
        Rvalue::Use(op) | Rvalue::Cast(_, op, _) => operand_known_len(st, op),
        Rvalue::Ref(_, _, place) | Rvalue::RawPtr(_, place) => {
            place_object_len(tcx, local_decls, st, *place)
        }
        _ => None,
    }
}

fn call_len<'tcx>(
    tcx: TyCtxt<'tcx>,
    local_decls: &LocalDecls<'tcx>,
    st: &IntervalState<'tcx>,
    term: &Terminator<'tcx>,
    path: &str,
    ptr_offset: Option<Interval>,
) -> Option<Interval> {
    let TerminatorKind::Call {
        args, destination, ..
    } = &term.kind
    else {
        return None;
    };

    if path.ends_with("::as_ptr") || path.ends_with("::as_mut_ptr") {
        return args
            .first()
            .and_then(|arg| operand_object_len(tcx, local_decls, st, &arg.node))
            .or_else(|| raw_ptr_pointee_len(tcx, destination.ty(local_decls, tcx).ty));
    }

    if (path.ends_with("::add") || path.ends_with("::wrapping_add") || path.ends_with("::offset"))
        && args.len() >= 2
    {
        let base = operand_known_len(st, &args[0].node)?;
        let offset = ptr_offset?;
        return Some(sub(&base, &offset));
    }

    if path.ends_with("::cast") {
        return args
            .first()
            .and_then(|arg| operand_known_len(st, &arg.node));
    }

    None
}

fn projected_places_with_runtime_index<'tcx>(
    st: &IntervalState<'tcx>,
    place: Place<'tcx>,
) -> Vec<Place<'tcx>> {
    st.all_fact_places()
        .into_iter()
        .filter(|candidate| {
            if place.local != candidate.local
                || place.projection.len() != candidate.projection.len()
            {
                return false;
            }
            place
                .projection
                .iter()
                .zip(candidate.projection.iter())
                .all(|(left, right)| match left {
                    ProjectionElem::Index(_) => {
                        matches!(right, ProjectionElem::ConstantIndex { .. })
                    }
                    _ => left == right,
                })
        })
        .collect()
}

// 当动态索引左值无法精确解析时，将其对应数组元素全部弱化为 Top。
fn eval_place<'tcx>(
    tcx: TyCtxt<'tcx>,
    local_decls: &LocalDecls<'tcx>,
    place: Place<'tcx>,
    st: &IntervalState<'tcx>,
) -> Interval {
    if let Some(resolved) = resolve_indexed_place(tcx, local_decls, st, place) {
        st.get_interval(&resolved)
    } else if has_runtime_index(place) {
        Interval::top()
    } else {
        st.get_interval(&place)
    }
}

// 将操作数求值为区间值。
pub(crate) fn eval_operand<'tcx>(
    tcx: TyCtxt<'tcx>,
    local_decls: &LocalDecls<'tcx>,
    op: &Operand<'tcx>,
    st: &IntervalState<'tcx>,
) -> Interval {
    match op {
        Operand::Copy(p) | Operand::Move(p) => eval_place(tcx, local_decls, *p, st),
        Operand::Constant(c) => interval_of_const(c),
    }
}

// 计算带溢出算术，并写入 (result, overflow) 元组区间。
fn eval_binary_op_with_overflow_interval<'tcx>(
    tcx: TyCtxt<'tcx>,
    st: &mut IntervalState<'tcx>,
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
    st.set_interval(result_place, result_sign);

    let overflow_place =
        place.project_deeper(&[ProjectionElem::Field(1u32.into(), tcx.types.bool)], tcx);
    st.set_interval(overflow_place, Interval::new(0, 0));
}

// 在区间域中计算二元运算。
fn eval_binary_op_interval<'tcx>(
    tcx: TyCtxt<'tcx>,
    st: &mut IntervalState<'tcx>,
    local_decls: &LocalDecls<'tcx>,
    op: &BinOp,
    ops: &Box<(Operand<'tcx>, Operand<'tcx>)>,
) -> Interval {
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
        _ => Interval::top(),
    }
}

// 在区间域中计算一元运算。
fn eval_unary_op_interval<'tcx>(
    tcx: TyCtxt<'tcx>,
    st: &mut IntervalState<'tcx>,
    local_decls: &LocalDecls<'tcx>,
    op: &UnOp,
    arg: &Operand<'tcx>,
) -> Interval {
    let sa = eval_operand(tcx, local_decls, arg, st);

    match op {
        UnOp::Neg => neg(&sa),
        _ => Interval::top(),
    }
}

// 对单条 MIR 语句执行区间与等价关系的 transfer。
pub fn transfer_stmt<'tcx>(
    tcx: TyCtxt<'tcx>,
    st: &mut IntervalState<'tcx>,
    stmt: &Statement<'tcx>,
    local_decls: &LocalDecls<'tcx>,
) {
    st.debug(format_args!("mir stmt: {:?}", stmt.kind));

    let kind = &stmt.kind;
    match kind {
        StatementKind::Assign(assign) => {
            let (place, rvalue) = &**assign;
            st.debug(format_args!("assign {:?} = {:?}", place, rvalue));
            let resolved_place = resolve_indexed_place(tcx, local_decls, st, *place);
            if resolved_place.is_none() {
                let targets = projected_places_with_runtime_index(st, *place);
                if targets.is_empty() {
                    return;
                }
                for p in targets {
                    st.debug(format_args!("weak update {:?} through runtime index", p));
                    st.set_interval(p, Interval::top());
                    st.clear_len(&p);
                }
                return;
            }
            let dst_place = resolved_place.unwrap_or(*place);
            let rhs_len = rvalue_len(tcx, local_decls, st, rvalue);
            st.clear_len(&dst_place);
            match rvalue {
                Rvalue::BinaryOp(op, ops) => match op {
                    BinOp::AddWithOverflow | BinOp::SubWithOverflow | BinOp::MulWithOverflow => {
                        eval_binary_op_with_overflow_interval(
                            tcx,
                            st,
                            &dst_place,
                            local_decls,
                            op,
                            ops,
                        );
                    }
                    _ => {
                        let rhs_interval = eval_binary_op_interval(tcx, st, local_decls, op, ops);
                        st.set_interval(dst_place, rhs_interval);
                    }
                },

                Rvalue::UnaryOp(UnOp::PtrMetadata, op) => {
                    let len_iv = match op {
                        Operand::Copy(src) | Operand::Move(src) => {
                            st.get_len(src).unwrap_or_else(Interval::top)
                        }
                        Operand::Constant(_) => Interval::top(),
                    };
                    st.set_interval(dst_place, len_iv);
                }

                Rvalue::UnaryOp(op, arg) => {
                    let rhs_interval = eval_unary_op_interval(tcx, st, local_decls, op, arg);
                    st.set_interval(dst_place, rhs_interval);
                }

                Rvalue::Use(op) => {
                    let rhs_interval = eval_operand(tcx, local_decls, op, st);
                    st.set_interval(dst_place, rhs_interval);
                }

                Rvalue::Cast(cast_kind, op, dst_ty) => {
                    let rhs_interval = eval_cast_interval(tcx, st, local_decls, op, *dst_ty);
                    st.set_interval(dst_place, rhs_interval);
                    if matches!(cast_kind, CastKind::PointerCoercion(_, _)) {
                        if let Operand::Copy(src) | Operand::Move(src) = op {
                            let src_ty = src.ty(local_decls, tcx).ty;
                            if let TyKind::Ref(_, inner, _) = src_ty.kind() {
                                if let TyKind::Array(_, len) = inner.kind() {
                                    if let Some(len) = len.try_to_target_usize(tcx) {
                                        st.set_len(
                                            dst_place,
                                            Interval::new(len as i128, len as i128),
                                        );
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
                            .map(|len| Interval::new(len as i128, len as i128))
                            .unwrap_or_else(Interval::top),
                        TyKind::Slice(_) => st.get_len(src).unwrap_or_else(Interval::top),
                        TyKind::Ref(_, inner, _) => match inner.kind() {
                            TyKind::Array(_, len) => len
                                .try_to_target_usize(tcx)
                                .map(|len| Interval::new(len as i128, len as i128))
                                .unwrap_or_else(Interval::top),
                            TyKind::Slice(_) => st.get_len(src).unwrap_or_else(Interval::top),
                            _ => Interval::top(),
                        },
                        _ => Interval::top(),
                    };
                    st.set_interval(dst_place, len_iv);
                }

                Rvalue::Aggregate(kind, indexvec) => match kind.as_ref() {
                    AggregateKind::Tuple => {
                        for (i, op) in indexvec.iter().enumerate() {
                            let elem_place = dst_place.project_deeper(
                                &[ProjectionElem::Field(i.into(), op.ty(local_decls, tcx))],
                                tcx,
                            );
                            let elem_interval = eval_operand(tcx, local_decls, op, st);
                            st.set_interval(elem_place, elem_interval);
                            st.clear_len(&elem_place);
                            if let Some(len) = operand_known_len(st, op) {
                                st.set_len(elem_place, len);
                            }
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
                            let elem_interval = eval_operand(tcx, local_decls, op, st);
                            st.set_interval(elem_place, elem_interval);
                            st.clear_len(&elem_place);
                            if let Some(len) = operand_known_len(st, op) {
                                st.set_len(elem_place, len);
                            }
                        }
                    }
                    _ => st.set_interval(dst_place, Interval::top()),
                },

                Rvalue::Ref(_region, _borrow_kind, borrowed_place) => {
                    let borrowed_interval = eval_place(tcx, local_decls, *borrowed_place, st);
                    st.set_interval(dst_place, borrowed_interval);
                    let borrowed_ty = borrowed_place.ty(local_decls, tcx).ty;
                    match borrowed_ty.kind() {
                        TyKind::Array(_, len) => {
                            if let Some(len) = len.try_to_target_usize(tcx) {
                                st.set_len(dst_place, Interval::new(len as i128, len as i128));
                            }
                        }
                        TyKind::Slice(_) => {
                            if let Some(len) = st.get_len(borrowed_place) {
                                st.set_len(dst_place, len);
                            }
                        }
                        _ => {}
                    }
                }

                _ => st.set_interval(dst_place, Interval::top()),
            }
            if let Some(len) = rhs_len {
                st.set_len(dst_place, len);
            }
            st.debug_map("stmt end map");
        }
        _ => {}
    }
}

pub fn transfer_terminator<'tcx>(
    tcx: TyCtxt<'tcx>,
    symbolic: &SymbolicState<'tcx>,
    st: &mut IntervalState<'tcx>,
    term: &Terminator<'tcx>,
    local_decls: &LocalDecls<'tcx>,
) {
    st.debug(format_args!("mir terminator: {:?}", term.kind));

    let TerminatorKind::Call {
        args, destination, ..
    } = &term.kind
    else {
        return;
    };
    st.clear_len(destination);
    let Some(path) = terminator_call_path(tcx, local_decls, term) else {
        return;
    };
    st.debug(format_args!("call {:?} := {path}", destination));
    let ptr_offset = if (path.ends_with("::add")
        || path.ends_with("::wrapping_add")
        || path.ends_with("::offset"))
        && args.len() >= 2
    {
        Some(eval_operand(tcx, local_decls, &args[1].node, st))
    } else {
        None
    };
    if let Some(len) = call_len(tcx, local_decls, st, term, &path, ptr_offset) {
        st.set_len(*destination, len);
    }
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
    for candidate in st.interval_places() {
        if !symbolic.equiv_places_readonly(receiver_place, candidate) {
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
        for candidate in st.interval_places() {
            if !symbolic.equiv_places_readonly(receiver_place, candidate) {
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
    st.set_interval(dst_deref, elem_iv);
    st.debug_map("terminator end map");
}
