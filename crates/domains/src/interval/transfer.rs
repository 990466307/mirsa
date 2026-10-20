use rustc_middle::mir::*;
use rustc_middle::ty::{FloatTy, Ty, TyCtxt, TyKind, TypingEnv};

use mirsa_framework::call_model::{PointerView, ViewLength};
use mirsa_relations::symbolic::SymbolicState;

use super::abstract_value::*;
use super::float_interval::{self, FloatCmpOp, FloatInterval, FloatKind};
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

pub fn scalar_value_place<'tcx>(
    tcx: TyCtxt<'tcx>,
    place: Place<'tcx>,
    ty: Ty<'tcx>,
) -> Option<Place<'tcx>> {
    let ty = tcx
        .try_normalize_erasing_regions(TypingEnv::fully_monomorphized(), ty)
        .ok()?;
    if scalar_layout(tcx, ty).is_some() {
        return Some(place);
    }
    let TyKind::Adt(adt, args) = ty.kind() else {
        return None;
    };
    if adt.variants().len() != 1 || adt.non_enum_variant().fields.len() != 1 {
        return None;
    }
    let field = &adt.non_enum_variant().fields[0usize.into()];
    let field_ty = tcx
        .try_normalize_erasing_regions(TypingEnv::fully_monomorphized(), field.ty(tcx, args))
        .ok()?;
    scalar_value_place(
        tcx,
        place.project_deeper(&[ProjectionElem::Field(0usize.into(), field_ty)], tcx),
        field_ty,
    )
}

fn is_scalar_interval_ty<'tcx>(tcx: TyCtxt<'tcx>, ty: Ty<'tcx>) -> bool {
    scalar_layout(tcx, ty).is_some()
}

pub fn float_kind_for_ty(ty: Ty<'_>) -> Option<FloatKind> {
    match ty.kind() {
        TyKind::Float(FloatTy::F32) => Some(FloatKind::F32),
        TyKind::Float(FloatTy::F64) => Some(FloatKind::F64),
        _ => None,
    }
}

fn operand_is_float_interval<'tcx>(
    tcx: TyCtxt<'tcx>,
    local_decls: &LocalDecls<'tcx>,
    op: &Operand<'tcx>,
) -> bool {
    float_kind_for_ty(op.ty(local_decls, tcx)).is_some()
}

fn place_is_float_interval<'tcx>(
    tcx: TyCtxt<'tcx>,
    local_decls: &LocalDecls<'tcx>,
    place: Place<'tcx>,
) -> bool {
    float_kind_for_ty(place.ty(local_decls, tcx).ty).is_some()
}

fn operand_is_scalar_interval<'tcx>(
    tcx: TyCtxt<'tcx>,
    local_decls: &LocalDecls<'tcx>,
    op: &Operand<'tcx>,
) -> bool {
    is_scalar_interval_ty(tcx, op.ty(local_decls, tcx))
}

fn place_is_scalar_interval<'tcx>(
    tcx: TyCtxt<'tcx>,
    local_decls: &LocalDecls<'tcx>,
    place: Place<'tcx>,
) -> bool {
    is_scalar_interval_ty(tcx, place.ty(local_decls, tcx).ty)
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

pub fn switch_value_to_i128<'tcx>(tcx: TyCtxt<'tcx>, ty: Ty<'tcx>, value: u128) -> Option<i128> {
    let (bit_width, signed) = scalar_layout(tcx, ty)?;
    if !signed && bit_width == 128 && value > i128::MAX as u128 {
        return None;
    }
    Some(bits_to_i128(value, bit_width, signed))
}

// 在区间域中计算 Cast；无法可靠保持顺序时保守回退为 Top。
fn eval_cast_interval<'tcx>(
    tcx: TyCtxt<'tcx>,
    st: &mut IntervalState<'tcx>,
    local_decls: &LocalDecls<'tcx>,
    op: &Operand<'tcx>,
    symbolic: &SymbolicState<'tcx>,
    dst_ty: Ty<'tcx>,
) -> Interval {
    let src_ty = op.ty(local_decls, tcx);
    if float_kind_for_ty(src_ty).is_some() {
        let src = eval_float_operand(tcx, local_decls, op, symbolic, st);
        return cast_float_to_integer(tcx, src, dst_ty);
    }
    let src_iv = eval_operand(tcx, local_decls, op, symbolic, st);
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

fn integer_bounds<'tcx>(tcx: TyCtxt<'tcx>, ty: Ty<'tcx>) -> Option<(i128, i128)> {
    let (bit_width, signed) = scalar_layout(tcx, ty)?;
    match ty.kind() {
        TyKind::Int(_) | TyKind::Uint(_) => {}
        _ => return None,
    }
    if signed {
        if bit_width == 128 {
            Some((i128::MIN, i128::MAX))
        } else {
            let high = (1i128 << (bit_width - 1)) - 1;
            Some((-high - 1, high))
        }
    } else if bit_width == 128 {
        // Integer intervals use i128 endpoints.  Values above i128::MAX are
        // conservatively represented by the upper sentinel.
        Some((0, i128::MAX))
    } else {
        Some((0, ((1u128 << bit_width) - 1) as i128))
    }
}

fn cast_float_value_to_integer<'tcx>(
    tcx: TyCtxt<'tcx>,
    value: f64,
    dst_ty: Ty<'tcx>,
) -> Option<i128> {
    let (low, high) = integer_bounds(tcx, dst_ty)?;
    let value = match dst_ty.kind() {
        TyKind::Int(_) => value as i128,
        TyKind::Uint(_) => {
            let value = value as u128;
            if value > i128::MAX as u128 {
                i128::MAX
            } else {
                value as i128
            }
        }
        _ => return None,
    };
    Some(value.clamp(low, high))
}

fn cast_float_to_integer<'tcx>(
    tcx: TyCtxt<'tcx>,
    src: FloatInterval,
    dst_ty: Ty<'tcx>,
) -> Interval {
    if src.is_bottom() {
        return Interval::empty();
    }
    let Some((dst_low, dst_high)) = integer_bounds(tcx, dst_ty) else {
        return Interval::top();
    };

    let mut low = i128::MAX;
    let mut high = i128::MIN;
    if src.has_numeric_values() {
        let Some(cast_low) = cast_float_value_to_integer(tcx, src.low, dst_ty) else {
            return Interval::top();
        };
        let Some(cast_high) = cast_float_value_to_integer(tcx, src.high, dst_ty) else {
            return Interval::top();
        };
        low = cast_low.min(cast_high).clamp(dst_low, dst_high);
        high = cast_low.max(cast_high).clamp(dst_low, dst_high);
    }
    if src.may_nan {
        low = low.min(0);
        high = high.max(0);
    }
    if low > high {
        Interval::empty()
    } else {
        Interval::new(low, high)
    }
}

fn eval_cast_float<'tcx>(
    tcx: TyCtxt<'tcx>,
    st: &mut IntervalState<'tcx>,
    local_decls: &LocalDecls<'tcx>,
    op: &Operand<'tcx>,
    symbolic: &SymbolicState<'tcx>,
    dst_ty: Ty<'tcx>,
) -> FloatInterval {
    let Some(dst_kind) = float_kind_for_ty(dst_ty) else {
        return FloatInterval::top();
    };
    let src_ty = op.ty(local_decls, tcx);
    if float_kind_for_ty(src_ty).is_some() {
        let src = eval_float_operand(tcx, local_decls, op, symbolic, st);
        if !src.has_numeric_values() {
            return FloatInterval::new(f64::INFINITY, f64::NEG_INFINITY, src.may_nan);
        }
        return FloatInterval::new(
            dst_kind.round(src.low),
            dst_kind.round(src.high),
            src.may_nan,
        );
    }

    let Some((src_bw, src_signed)) = scalar_layout(tcx, src_ty) else {
        return FloatInterval::top();
    };
    if !matches!(src_ty.kind(), TyKind::Int(_) | TyKind::Uint(_)) {
        return FloatInterval::top();
    }
    let src = eval_operand(tcx, local_decls, op, symbolic, st);
    if src.is_empty() {
        return FloatInterval::bottom();
    }
    let Some((type_low, type_high)) = integer_bounds(tcx, src_ty) else {
        return FloatInterval::top();
    };
    let low = src.low.max(type_low);
    let high = src.high.min(type_high);
    if !src_signed && src_bw == 128 && src.high == i128::MAX {
        // Preserve the portion of u128 that the integer interval sentinel
        // cannot express precisely.
        return FloatInterval::numeric(
            cast_integer_endpoint_to_float(dst_kind, low, src_signed),
            cast_u128_to_float(dst_kind, u128::MAX),
        );
    }
    FloatInterval::numeric(
        cast_integer_endpoint_to_float(dst_kind, low, src_signed),
        cast_integer_endpoint_to_float(dst_kind, high, src_signed),
    )
}

fn cast_integer_endpoint_to_float(kind: FloatKind, value: i128, signed: bool) -> f64 {
    if signed {
        match kind {
            FloatKind::F32 => f64::from(value as f32),
            FloatKind::F64 => value as f64,
        }
    } else {
        cast_u128_to_float(kind, value.max(0) as u128)
    }
}

fn cast_u128_to_float(kind: FloatKind, value: u128) -> f64 {
    match kind {
        FloatKind::F32 => f64::from(value as f32),
        FloatKind::F64 => value as f64,
    }
}

// 将 MIR 常量操作数求值为区间值。
pub fn interval_of_const<'tcx>(tcx: TyCtxt<'tcx>, c: &ConstOperand<'tcx>) -> Interval {
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
    if let Some(si) = k.try_eval_scalar_int(tcx, TypingEnv::fully_monomorphized()) {
        let bit_width = match ty.kind() {
            TyKind::Int(_) | TyKind::Uint(_) => si.size().bits(),
            _ => bit_width_from_ty,
        };
        if !signed && bit_width == 128 && si.to_bits_unchecked() > i128::MAX as u128 {
            return Interval::top();
        }
        let v = bits_to_i128(si.to_bits_unchecked(), bit_width, signed);
        Interval::new(v, v)
    } else {
        Interval::top()
    }
}

pub fn float_interval_of_const<'tcx>(tcx: TyCtxt<'tcx>, c: &ConstOperand<'tcx>) -> FloatInterval {
    let Some(kind) = float_kind_for_ty(c.ty()) else {
        return FloatInterval::top();
    };
    let Some(scalar) = c
        .const_
        .try_eval_scalar_int(tcx, TypingEnv::fully_monomorphized())
    else {
        return FloatInterval::top();
    };
    let bits = scalar.to_bits_unchecked();
    let value = match kind {
        FloatKind::F32 => f64::from(f32::from_bits(bits as u32)),
        FloatKind::F64 => f64::from_bits(bits as u64),
    };
    FloatInterval::singleton(value)
}

// 判断 place 是否包含运行时索引投影。
fn has_runtime_index<'tcx>(place: Place<'tcx>) -> bool {
    place
        .projection
        .iter()
        .any(|elem| matches!(elem, ProjectionElem::Index(_)))
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
    if let Some(len) = static_len(tcx, ty) {
        return len;
    }
    if let Some(len) = st.get_len(&place) {
        return len;
    }
    match ty.kind() {
        TyKind::Slice(_) => Interval::top(),
        TyKind::Ref(_, inner, _) if matches!(inner.kind(), TyKind::Slice(_)) => Interval::top(),
        _ => static_len(tcx, ty).unwrap_or_else(Interval::top),
    }
}

pub fn operand_len<'tcx>(
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

pub fn operand_known_len<'tcx>(st: &IntervalState<'tcx>, op: &Operand<'tcx>) -> Option<Interval> {
    match op {
        Operand::Copy(place) | Operand::Move(place) => st.get_len(place),
        Operand::Constant(c) => match c.const_ {
            Const::Val(ConstValue::Slice { meta, .. }, _) => {
                Some(Interval::new(meta as i128, meta as i128))
            }
            _ => None,
        },
    }
}

fn slice_or_array_len<'tcx>(
    tcx: TyCtxt<'tcx>,
    local_decls: &LocalDecls<'tcx>,
    st: &mut IntervalState<'tcx>,
    symbolic: &SymbolicState<'tcx>,
    place: Place<'tcx>,
) -> Option<Interval> {
    let ty = place.ty(local_decls, tcx).ty;
    match ty.kind() {
        TyKind::Array(_, len) => len
            .try_to_target_usize(tcx)
            .map(|len| Interval::new(len as i128, len as i128)),
        TyKind::Slice(_) => Some(st.read_len_resolved_or_top(symbolic, place)),
        TyKind::Ref(_, inner, _) => match inner.kind() {
            TyKind::Array(_, len) => len
                .try_to_target_usize(tcx)
                .map(|len| Interval::new(len as i128, len as i128)),
            TyKind::Slice(_) => Some(st.read_len_resolved_or_top(symbolic, place)),
            _ => None,
        },
        _ => None,
    }
}

fn operand_slice_or_array_len<'tcx>(
    tcx: TyCtxt<'tcx>,
    local_decls: &LocalDecls<'tcx>,
    st: &mut IntervalState<'tcx>,
    symbolic: &SymbolicState<'tcx>,
    op: &Operand<'tcx>,
) -> Option<Interval> {
    match op {
        Operand::Copy(place) | Operand::Move(place) => {
            slice_or_array_len(tcx, local_decls, st, symbolic, *place)
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
    static_len(tcx, place.ty(local_decls, tcx).ty).or_else(|| st.get_len(&place))
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
    symbolic: &SymbolicState<'tcx>,
    rvalue: &Rvalue<'tcx>,
) -> Option<Interval> {
    match rvalue {
        Rvalue::Use(op) | Rvalue::Cast(_, op, _) => {
            let static_length = match rvalue {
                Rvalue::Cast(CastKind::PointerCoercion(_, _), _, _) => {
                    static_len(tcx, op.ty(local_decls, tcx))
                }
                _ => None,
            };
            static_length
                .or_else(|| {
                    operand_place(op).and_then(|place| st.tracked_len_resolved(symbolic, place))
                })
                .or_else(|| operand_known_len(st, op))
        }
        Rvalue::Ref(_, _, place) | Rvalue::RawPtr(_, place) => {
            if let Some(len) = static_len(tcx, place.ty(local_decls, tcx).ty) {
                return Some(len);
            }
            if matches!(
                place.ty(local_decls, tcx).ty.kind(),
                TyKind::Slice(_) | TyKind::Str
            ) {
                if let Some((ProjectionElem::Deref, prefix)) = place.projection.split_last() {
                    let pointer = Place::from(place.local).project_deeper(prefix, tcx);
                    return st.tracked_len_resolved(symbolic, pointer);
                }
            }
            if let Some(len) = st
                .tracked_len_resolved(symbolic, *place)
                .filter(|len| !len.is_empty())
            {
                return Some(len);
            }
            None
        }
        _ => None,
    }
}

fn call_len<'tcx>(
    tcx: TyCtxt<'tcx>,
    local_decls: &LocalDecls<'tcx>,
    st: &mut IntervalState<'tcx>,
    symbolic: &SymbolicState<'tcx>,
    term: &Terminator<'tcx>,
    path: &str,
    ptr_offset: Option<Interval>,
) -> Option<Interval> {
    let TerminatorKind::Call {
        args,
        destination,
        func,
        ..
    } = &term.kind
    else {
        return None;
    };

    if let Some(view) = PointerView::classify(tcx, func, local_decls) {
        return match view.length {
            ViewLength::Argument(index) => args
                .get(index)
                .map(|arg| eval_operand(tcx, local_decls, &arg.node, symbolic, st)),
            ViewLength::One => Some(Interval::new(1, 1)),
            ViewLength::Unknown => None,
            ViewLength::Preserve => {
                let source = view.source(tcx, local_decls, &args.first()?.node)?;
                static_len(tcx, source.ty(local_decls, tcx))
                    .or_else(|| {
                        source
                            .place()
                            .and_then(|place| st.tracked_len_resolved(symbolic, place))
                    })
                    .or_else(|| operand_known_len(st, &source))
            }
        };
    }

    let standard = matches!(func.ty(local_decls, tcx).kind(), TyKind::FnDef(callee, _)
        if !callee.is_local() && matches!(tcx.crate_name(callee.krate).as_str(), "core" | "alloc" | "std"));
    if standard
        && [
            "::as_bytes",
            "::as_bytes_mut",
            "::as_str",
            "::as_mut_str",
            "::from_utf8_unchecked",
            "::from_utf8_unchecked_mut",
        ]
        .iter()
        .any(|suffix| path.ends_with(suffix))
    {
        return args
            .first()
            .and_then(|arg| operand_known_len(st, &arg.node));
    }

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

fn call_interval<'tcx>(
    tcx: TyCtxt<'tcx>,
    local_decls: &LocalDecls<'tcx>,
    term: &Terminator<'tcx>,
    path: &str,
) -> Option<Interval> {
    let TerminatorKind::Call { func, .. } = &term.kind else {
        return None;
    };
    let TyKind::FnDef(_, args) = func.ty(local_decls, tcx).kind() else {
        return None;
    };
    if path.ends_with("::size_of") {
        let ty = args.types().next()?;
        let layout = tcx
            .layout_of(TypingEnv::fully_monomorphized().as_query_input(ty))
            .ok()?;
        let size = layout.size.bytes() as i128;
        return Some(Interval::new(size, size));
    }
    if path.ends_with("::align_of") {
        let ty = args.types().next()?;
        let layout = tcx
            .layout_of(TypingEnv::fully_monomorphized().as_query_input(ty))
            .ok()?;
        let align = layout.align.abi.bytes() as i128;
        return Some(Interval::new(align, align));
    }

    None
}

fn eval_place<'tcx>(
    symbolic: &SymbolicState<'tcx>,
    place: Place<'tcx>,
    st: &mut IntervalState<'tcx>,
) -> Interval {
    if has_runtime_index(place) {
        Interval::top()
    } else {
        st.read_interval_resolved(symbolic, place)
    }
}

fn eval_float_place<'tcx>(
    symbolic: &SymbolicState<'tcx>,
    place: Place<'tcx>,
    st: &mut IntervalState<'tcx>,
) -> FloatInterval {
    if has_runtime_index(place) {
        FloatInterval::top()
    } else {
        st.read_float_interval_resolved(symbolic, place)
    }
}

// 将操作数求值为区间值。
pub fn eval_operand<'tcx>(
    tcx: TyCtxt<'tcx>,
    local_decls: &LocalDecls<'tcx>,
    op: &Operand<'tcx>,
    symbolic: &SymbolicState<'tcx>,
    st: &mut IntervalState<'tcx>,
) -> Interval {
    if !operand_is_scalar_interval(tcx, local_decls, op) {
        return Interval::top();
    }
    match op {
        Operand::Copy(p) | Operand::Move(p) => eval_place(symbolic, *p, st),
        Operand::Constant(c) => interval_of_const(tcx, c),
    }
}

pub fn eval_float_operand<'tcx>(
    tcx: TyCtxt<'tcx>,
    local_decls: &LocalDecls<'tcx>,
    op: &Operand<'tcx>,
    symbolic: &SymbolicState<'tcx>,
    st: &mut IntervalState<'tcx>,
) -> FloatInterval {
    if !operand_is_float_interval(tcx, local_decls, op) {
        return FloatInterval::top();
    }
    match op {
        Operand::Copy(place) | Operand::Move(place) => eval_float_place(symbolic, *place, st),
        Operand::Constant(c) => float_interval_of_const(tcx, c),
    }
}

pub fn eval_checked_binary_interval<'tcx>(
    tcx: TyCtxt<'tcx>,
    st: &mut IntervalState<'tcx>,
    symbolic: &SymbolicState<'tcx>,
    local_decls: &LocalDecls<'tcx>,
    op: &BinOp,
    a: &Operand<'tcx>,
    b: &Operand<'tcx>,
) -> (Interval, Interval) {
    let sa = eval_operand(tcx, local_decls, a, symbolic, st);
    let sb = eval_operand(tcx, local_decls, b, symbolic, st);

    let operand_ty = a.ty(local_decls, tcx);
    if matches!(op, BinOp::SubWithOverflow)
        && same_integer_operand(tcx, local_decls, symbolic, a, b)
    {
        (Interval::new(0, 0), Interval::new(0, 0))
    } else if let Some((bits, signed)) = scalar_layout(tcx, operand_ty) {
        super::integer_arithmetic::arithmetic(*op, sa, sb, bits, signed)
    } else {
        (Interval::top(), Interval::new(0, 1))
    }
}

// 计算带溢出算术，并写入 (result, overflow) 元组区间。
fn eval_binary_op_with_overflow_interval<'tcx>(
    tcx: TyCtxt<'tcx>,
    st: &mut IntervalState<'tcx>,
    symbolic: &SymbolicState<'tcx>,
    place: &Place<'tcx>,
    local_decls: &LocalDecls<'tcx>,
    op: &BinOp,
    ops: &Box<(Operand<'tcx>, Operand<'tcx>)>,
) {
    let (a, b) = &**ops;
    let operand_ty = a.ty(local_decls, tcx);
    let (result_sign, overflow) =
        eval_checked_binary_interval(tcx, st, symbolic, local_decls, op, a, b);

    let result_place = place.project_deeper(&[ProjectionElem::Field(0u32.into(), operand_ty)], tcx);
    st.set_interval_resolved(symbolic, result_place, result_sign);

    let overflow_place =
        place.project_deeper(&[ProjectionElem::Field(1u32.into(), tcx.types.bool)], tcx);
    st.set_interval_resolved(symbolic, overflow_place, overflow);
}

// 在区间域中计算二元运算。
fn same_integer_operand<'tcx>(
    tcx: TyCtxt<'tcx>,
    local_decls: &LocalDecls<'tcx>,
    symbolic: &SymbolicState<'tcx>,
    a: &Operand<'tcx>,
    b: &Operand<'tcx>,
) -> bool {
    if !matches!(
        a.ty(local_decls, tcx).kind(),
        TyKind::Int(_) | TyKind::Uint(_)
    ) || a.ty(local_decls, tcx) != b.ty(local_decls, tcx)
    {
        return false;
    }
    match (a.place(), b.place()) {
        (Some(a), Some(b)) => symbolic.equiv_places_readonly(a, b),
        _ => false,
    }
}

fn eval_binary_op_interval<'tcx>(
    tcx: TyCtxt<'tcx>,
    st: &mut IntervalState<'tcx>,
    symbolic: &SymbolicState<'tcx>,
    local_decls: &LocalDecls<'tcx>,
    op: &BinOp,
    ops: &Box<(Operand<'tcx>, Operand<'tcx>)>,
) -> Interval {
    let (a, b) = &**ops;
    let is_comparison = matches!(
        op,
        BinOp::Le | BinOp::Lt | BinOp::Ge | BinOp::Gt | BinOp::Eq | BinOp::Ne
    );
    if is_comparison
        && operand_is_float_interval(tcx, local_decls, a)
        && operand_is_float_interval(tcx, local_decls, b)
    {
        let left = eval_float_operand(tcx, local_decls, a, symbolic, st);
        let right = eval_float_operand(tcx, local_decls, b, symbolic, st);
        let op = match op {
            BinOp::Lt => FloatCmpOp::Lt,
            BinOp::Le => FloatCmpOp::Le,
            BinOp::Gt => FloatCmpOp::Gt,
            BinOp::Ge => FloatCmpOp::Ge,
            BinOp::Eq => FloatCmpOp::Eq,
            BinOp::Ne => FloatCmpOp::Ne,
            _ => unreachable!(),
        };
        return float_interval::compare(op, &left, &right);
    }
    if is_comparison
        && (!operand_is_scalar_interval(tcx, local_decls, a)
            || !operand_is_scalar_interval(tcx, local_decls, b))
    {
        return Interval::new(0, 1);
    }

    if is_comparison {
        if let Some(value) = symbolic.compare(tcx, local_decls, *op, a, b) {
            return Interval::new(i128::from(value), i128::from(value));
        }
    }

    let sa = eval_operand(tcx, local_decls, a, symbolic, st);
    let sb = eval_operand(tcx, local_decls, b, symbolic, st);

    if matches!(op, BinOp::Sub) && same_integer_operand(tcx, local_decls, symbolic, a, b) {
        return Interval::new(0, 0);
    }
    if matches!(op, BinOp::Add | BinOp::Sub | BinOp::Mul) {
        return scalar_layout(tcx, a.ty(local_decls, tcx))
            .map(|(bits, signed)| {
                super::integer_arithmetic::arithmetic(*op, sa, sb, bits, signed).0
            })
            .unwrap_or_else(Interval::top);
    }
    match op {
        BinOp::Div => div(&sa, &sb),
        BinOp::Rem => rem(&sa, &sb),
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

fn eval_binary_op_float<'tcx>(
    tcx: TyCtxt<'tcx>,
    st: &mut IntervalState<'tcx>,
    symbolic: &SymbolicState<'tcx>,
    local_decls: &LocalDecls<'tcx>,
    op: &BinOp,
    ops: &Box<(Operand<'tcx>, Operand<'tcx>)>,
) -> FloatInterval {
    let (a, b) = &**ops;
    let Some(kind) = float_kind_for_ty(a.ty(local_decls, tcx)) else {
        return FloatInterval::top();
    };
    let left = eval_float_operand(tcx, local_decls, a, symbolic, st);
    let right = eval_float_operand(tcx, local_decls, b, symbolic, st);
    match op {
        BinOp::Add => float_interval::add(kind, &left, &right),
        BinOp::Sub => float_interval::sub(kind, &left, &right),
        BinOp::Mul => float_interval::mul(kind, &left, &right),
        BinOp::Div => float_interval::div(kind, &left, &right),
        _ => FloatInterval::top(),
    }
}

// 在区间域中计算一元运算。
fn eval_unary_op_interval<'tcx>(
    tcx: TyCtxt<'tcx>,
    st: &mut IntervalState<'tcx>,
    symbolic: &SymbolicState<'tcx>,
    local_decls: &LocalDecls<'tcx>,
    op: &UnOp,
    arg: &Operand<'tcx>,
) -> Interval {
    let sa = eval_operand(tcx, local_decls, arg, symbolic, st);

    match op {
        UnOp::Neg => scalar_layout(tcx, arg.ty(local_decls, tcx))
            .map(|(bits, signed)| {
                super::integer_arithmetic::arithmetic(
                    BinOp::Sub,
                    Interval::new(0, 0),
                    sa,
                    bits,
                    signed,
                )
                .0
            })
            .unwrap_or_else(Interval::top),
        _ => Interval::top(),
    }
}

fn eval_unary_op_float<'tcx>(
    tcx: TyCtxt<'tcx>,
    st: &mut IntervalState<'tcx>,
    symbolic: &SymbolicState<'tcx>,
    local_decls: &LocalDecls<'tcx>,
    op: &UnOp,
    arg: &Operand<'tcx>,
) -> FloatInterval {
    let value = eval_float_operand(tcx, local_decls, arg, symbolic, st);
    match op {
        UnOp::Neg => float_interval::neg(&value),
        _ => FloatInterval::top(),
    }
}

pub fn eval_assign_rhs_interval<'tcx>(
    tcx: TyCtxt<'tcx>,
    st: &mut IntervalState<'tcx>,
    symbolic: &SymbolicState<'tcx>,
    local_decls: &LocalDecls<'tcx>,
    rvalue: &Rvalue<'tcx>,
) -> Interval {
    match rvalue {
        Rvalue::NullaryOp(op @ (NullOp::SizeOf | NullOp::AlignOf), ty) => tcx
            .layout_of(TypingEnv::fully_monomorphized().as_query_input(*ty))
            .ok()
            .map(|layout| {
                let bytes = match op {
                    NullOp::SizeOf => layout.size.bytes(),
                    _ => layout.align.abi.bytes(),
                } as i128;
                Interval::new(bytes, bytes)
            })
            .unwrap_or_else(Interval::top),
        Rvalue::BinaryOp(op, ops) => match op {
            BinOp::AddWithOverflow | BinOp::SubWithOverflow | BinOp::MulWithOverflow => {
                Interval::top()
            }
            _ => eval_binary_op_interval(tcx, st, symbolic, local_decls, op, ops),
        },
        Rvalue::UnaryOp(UnOp::PtrMetadata, op) => match op {
            Operand::Copy(src) | Operand::Move(src) => st.read_len_resolved_or_top(symbolic, *src),
            Operand::Constant(_) => Interval::top(),
        },
        Rvalue::UnaryOp(op, arg) => eval_unary_op_interval(tcx, st, symbolic, local_decls, op, arg),
        Rvalue::Use(op) => eval_operand(tcx, local_decls, op, symbolic, st),
        Rvalue::Cast(_, op, dst_ty) => {
            eval_cast_interval(tcx, st, local_decls, op, symbolic, *dst_ty)
        }
        Rvalue::Len(src) => {
            let src_ty = src.ty(local_decls, tcx).ty;
            match src_ty.kind() {
                TyKind::Array(_, len) => len
                    .try_to_target_usize(tcx)
                    .map(|len| Interval::new(len as i128, len as i128))
                    .unwrap_or_else(Interval::top),
                TyKind::Slice(_) => st.read_len_resolved_or_top(symbolic, *src),
                TyKind::Ref(_, inner, _) => match inner.kind() {
                    TyKind::Array(_, len) => len
                        .try_to_target_usize(tcx)
                        .map(|len| Interval::new(len as i128, len as i128))
                        .unwrap_or_else(Interval::top),
                    TyKind::Slice(_) => st.read_len_resolved_or_top(symbolic, *src),
                    _ => Interval::top(),
                },
                _ => Interval::top(),
            }
        }
        _ => Interval::top(),
    }
}

pub fn eval_assign_rhs_float<'tcx>(
    tcx: TyCtxt<'tcx>,
    st: &mut IntervalState<'tcx>,
    symbolic: &SymbolicState<'tcx>,
    local_decls: &LocalDecls<'tcx>,
    rvalue: &Rvalue<'tcx>,
) -> FloatInterval {
    match rvalue {
        Rvalue::BinaryOp(op, ops) => eval_binary_op_float(tcx, st, symbolic, local_decls, op, ops),
        Rvalue::UnaryOp(op, arg) => eval_unary_op_float(tcx, st, symbolic, local_decls, op, arg),
        Rvalue::Use(op) => eval_float_operand(tcx, local_decls, op, symbolic, st),
        Rvalue::Cast(_, op, dst_ty) => eval_cast_float(tcx, st, local_decls, op, symbolic, *dst_ty),
        _ => FloatInterval::top(),
    }
}

// 对单条 MIR 语句执行区间与等价关系的 transfer。
fn assign_operand_value<'tcx>(
    tcx: TyCtxt<'tcx>,
    symbolic: &SymbolicState<'tcx>,
    st: &mut IntervalState<'tcx>,
    source: &IntervalState<'tcx>,
    local_decls: &LocalDecls<'tcx>,
    destination: Place<'tcx>,
    operand: &Operand<'tcx>,
) {
    let mut values = source.clone();
    if place_is_scalar_interval(tcx, local_decls, destination) {
        let bits = match operand {
            Operand::Constant(c) => c
                .const_
                .try_eval_scalar_int(tcx, TypingEnv::fully_monomorphized()),
            Operand::Copy(place) | Operand::Move(place) => {
                source.integer_bits_resolved(symbolic, *place)
            }
        };
        if let Some(bits) = bits {
            st.set_integer_bits_resolved(symbolic, destination, bits);
        }
    }
    if place_is_scalar_interval(tcx, local_decls, destination) {
        let value = eval_operand(tcx, local_decls, operand, symbolic, &mut values);
        st.set_interval_resolved(symbolic, destination, value);
    }
    if place_is_float_interval(tcx, local_decls, destination) {
        let value = eval_float_operand(tcx, local_decls, operand, symbolic, &mut values);
        st.set_float_interval_resolved(symbolic, destination, value);
    }
    if let Some(source_place) = operand_place(operand) {
        if let Some(variant) = source.enum_variant_resolved(symbolic, source_place) {
            st.set_enum_variant_resolved(symbolic, destination, variant);
        }
        if let (Some(destination), Some(origin)) = (
            symbolic.normalize_place(destination),
            symbolic.normalize_place(source_place),
        ) {
            st.copy_children_from(source, &destination, &origin);
        }
    }
    st.clear_len_resolved(symbolic, &destination);
    if let Some(len) = operand_known_len(source, operand) {
        st.set_len_resolved(symbolic, destination, len);
    }
    if let Operand::Constant(constant) = operand {
        super::constant_bytes::assign(tcx, symbolic, st, destination, constant);
    }
}

pub fn transfer_stmt<'tcx>(
    tcx: TyCtxt<'tcx>,
    symbolic: &SymbolicState<'tcx>,
    st: &mut IntervalState<'tcx>,
    stmt: &Statement<'tcx>,
    local_decls: &LocalDecls<'tcx>,
) {
    let kind = &stmt.kind;
    match kind {
        StatementKind::Assign(assign) => {
            let (place, rvalue) = &**assign;
            let source_state = st.clone();
            if let Some(path) = symbolic.normalize_place(*place) {
                st.invalidate_value_facts(symbolic, &[path]);
            }
            st.debug(format_args!("stmt assign {:?} = {:?}", place, rvalue));
            if let Rvalue::Use(op) = rvalue {
                if operand_place(op).is_some_and(has_runtime_index)
                    && !has_runtime_index(*place)
                    && (place_is_scalar_interval(tcx, local_decls, *place)
                        || place_is_float_interval(tcx, local_decls, *place))
                {
                    return;
                }
            }
            if has_runtime_index(*place) {
                return;
            }
            let dst_place = *place;
            if let Some(path) = symbolic.normalize_place(dst_place) {
                st.invalidate_children(&path);
            }
            let rhs_len = rvalue_len(tcx, local_decls, st, symbolic, rvalue);
            st.clear_len_resolved(symbolic, &dst_place);
            let tracks_interval_value = place_is_scalar_interval(tcx, local_decls, dst_place);
            let tracks_float_value = place_is_float_interval(tcx, local_decls, dst_place);
            match rvalue {
                Rvalue::NullaryOp(NullOp::SizeOf | NullOp::AlignOf, _) => {
                    let value = eval_assign_rhs_interval(tcx, st, symbolic, local_decls, rvalue);
                    st.set_interval_resolved(symbolic, dst_place, value);
                }
                Rvalue::BinaryOp(op, ops) => match op {
                    BinOp::AddWithOverflow | BinOp::SubWithOverflow | BinOp::MulWithOverflow => {
                        eval_binary_op_with_overflow_interval(
                            tcx,
                            st,
                            symbolic,
                            &dst_place,
                            local_decls,
                            op,
                            ops,
                        );
                    }
                    _ => {
                        if tracks_interval_value {
                            let rhs_interval =
                                eval_binary_op_interval(tcx, st, symbolic, local_decls, op, ops);
                            st.set_interval_resolved(symbolic, dst_place, rhs_interval);
                        }
                        if tracks_float_value {
                            let rhs_interval =
                                eval_binary_op_float(tcx, st, symbolic, local_decls, op, ops);
                            st.set_float_interval_resolved(symbolic, dst_place, rhs_interval);
                        }
                    }
                },

                Rvalue::UnaryOp(UnOp::PtrMetadata, op) => {
                    let len_iv = match op {
                        Operand::Copy(src) | Operand::Move(src) => {
                            st.read_len_resolved_or_top(symbolic, *src)
                        }
                        Operand::Constant(_) => Interval::top(),
                    };
                    if tracks_interval_value {
                        st.set_interval_resolved(symbolic, dst_place, len_iv);
                    }
                }

                Rvalue::UnaryOp(op, arg) => {
                    if tracks_interval_value {
                        let rhs_interval =
                            eval_unary_op_interval(tcx, st, symbolic, local_decls, op, arg);
                        st.set_interval_resolved(symbolic, dst_place, rhs_interval);
                    }
                    if tracks_float_value {
                        let rhs_interval =
                            eval_unary_op_float(tcx, st, symbolic, local_decls, op, arg);
                        st.set_float_interval_resolved(symbolic, dst_place, rhs_interval);
                    }
                }

                Rvalue::Use(op) => {
                    assign_operand_value(
                        tcx,
                        symbolic,
                        st,
                        &source_state,
                        local_decls,
                        dst_place,
                        op,
                    );
                }

                Rvalue::Cast(_, op, dst_ty) => {
                    if tracks_interval_value {
                        let rhs_interval =
                            eval_cast_interval(tcx, st, local_decls, op, symbolic, *dst_ty);
                        st.set_interval_resolved(symbolic, dst_place, rhs_interval);
                    }
                    if tracks_float_value {
                        let rhs_interval =
                            eval_cast_float(tcx, st, local_decls, op, symbolic, *dst_ty);
                        st.set_float_interval_resolved(symbolic, dst_place, rhs_interval);
                    }
                }

                Rvalue::Len(src) => {
                    let src_ty = src.ty(local_decls, tcx).ty;
                    let len_iv = match src_ty.kind() {
                        TyKind::Array(_, len) => len
                            .try_to_target_usize(tcx)
                            .map(|len| Interval::new(len as i128, len as i128))
                            .unwrap_or_else(Interval::top),
                        TyKind::Slice(_) => st.read_len_resolved_or_top(symbolic, *src),
                        TyKind::Ref(_, inner, _) => match inner.kind() {
                            TyKind::Array(_, len) => len
                                .try_to_target_usize(tcx)
                                .map(|len| Interval::new(len as i128, len as i128))
                                .unwrap_or_else(Interval::top),
                            TyKind::Slice(_) => st.read_len_resolved_or_top(symbolic, *src),
                            _ => Interval::top(),
                        },
                        _ => Interval::top(),
                    };
                    st.set_interval_resolved(symbolic, dst_place, len_iv);
                }

                Rvalue::Aggregate(kind, indexvec) => match kind.as_ref() {
                    AggregateKind::Tuple | AggregateKind::Adt(..) => {
                        if let AggregateKind::Adt(def_id, variant, ..) = kind.as_ref() {
                            if tcx.adt_def(*def_id).is_enum() {
                                st.set_enum_variant_resolved(symbolic, dst_place, variant.index());
                            }
                        }
                        let base = match kind.as_ref() {
                            AggregateKind::Adt(def_id, variant, ..)
                                if tcx.adt_def(*def_id).is_enum() =>
                            {
                                dst_place.project_deeper(
                                    &[ProjectionElem::Downcast(None, *variant)],
                                    tcx,
                                )
                            }
                            _ => dst_place,
                        };
                        for (i, op) in indexvec.iter().enumerate() {
                            let elem_place = base.project_deeper(
                                &[ProjectionElem::Field(i.into(), op.ty(local_decls, tcx))],
                                tcx,
                            );
                            assign_operand_value(
                                tcx,
                                symbolic,
                                st,
                                &source_state,
                                local_decls,
                                elem_place,
                                op,
                            );
                        }
                    }

                    AggregateKind::Array(_elem_ty) => {
                        st.clear_index_summaries(symbolic, dst_place);
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
                            let elem_interval = eval_operand(tcx, local_decls, op, symbolic, st);
                            if place_is_scalar_interval(tcx, local_decls, elem_place) {
                                st.set_interval_resolved(symbolic, elem_place, elem_interval);
                            }
                            if place_is_float_interval(tcx, local_decls, elem_place) {
                                let elem_interval =
                                    eval_float_operand(tcx, local_decls, op, symbolic, st);
                                st.set_float_interval_resolved(symbolic, elem_place, elem_interval);
                            }
                            st.clear_len_resolved(symbolic, &elem_place);
                            if let Some(len) = operand_known_len(st, op) {
                                st.set_len_resolved(symbolic, elem_place, len);
                            }
                        }
                    }
                    _ => {
                        if tracks_interval_value {
                            st.set_interval_resolved(symbolic, dst_place, Interval::top());
                        }
                        if tracks_float_value {
                            st.set_float_interval_resolved(
                                symbolic,
                                dst_place,
                                FloatInterval::top(),
                            );
                        }
                    }
                },

                Rvalue::Ref(_region, _borrow_kind, borrowed_place) => {
                    let borrowed_ty = borrowed_place.ty(local_decls, tcx).ty;
                    match borrowed_ty.kind() {
                        TyKind::Array(_, len) => {
                            if let Some(len) = len.try_to_target_usize(tcx) {
                                st.set_len_resolved(
                                    symbolic,
                                    dst_place,
                                    Interval::new(len as i128, len as i128),
                                );
                            }
                        }
                        TyKind::Slice(_) => {
                            let len = st.read_len_resolved_or_top(symbolic, *borrowed_place);
                            st.set_len_resolved(symbolic, dst_place, len);
                        }
                        _ => {}
                    }
                }

                _ => {
                    if tracks_interval_value {
                        st.set_interval_resolved(symbolic, dst_place, Interval::top());
                    }
                    if tracks_float_value {
                        st.set_float_interval_resolved(symbolic, dst_place, FloatInterval::top());
                    }
                }
            }
            if let Some(len) = rhs_len {
                st.set_len_resolved(symbolic, dst_place, len);
            }
            if let Rvalue::BinaryOp(BinOp::Shl | BinOp::Shr, operands) = rvalue {
                let get_bits = |operand: &Operand<'tcx>| match operand {
                    Operand::Constant(c) => c
                        .const_
                        .try_eval_scalar_int(tcx, TypingEnv::fully_monomorphized()),
                    Operand::Copy(p) | Operand::Move(p) => {
                        source_state.integer_bits_resolved(symbolic, *p)
                    }
                };
                if let (Some(left), Some(right), Some((width, signed))) = (
                    get_bits(&operands.0),
                    get_bits(&operands.1),
                    scalar_layout(tcx, dst_place.ty(local_decls, tcx).ty),
                ) {
                    let shift = right.to_bits_unchecked();
                    if shift < width as u128 {
                        let bits = if matches!(rvalue, Rvalue::BinaryOp(BinOp::Shl, _)) {
                            left.to_bits_unchecked() << shift
                        } else if signed {
                            (left.to_int(left.size()) >> shift) as u128
                        } else {
                            left.to_bits_unchecked() >> shift
                        };
                        let value =
                            rustc_middle::ty::ScalarInt::truncate_from_uint(bits, left.size()).0;
                        st.set_integer_bits_resolved(symbolic, dst_place, value);
                    }
                }
            }
        }
        StatementKind::SetDiscriminant {
            place,
            variant_index,
        } => {
            st.set_enum_variant_resolved(symbolic, **place, variant_index.index());
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
    let TerminatorKind::Call {
        args, destination, ..
    } = &term.kind
    else {
        return;
    };
    if let Some(path) = symbolic.normalize_place(*destination) {
        st.invalidate_paths(symbolic, &[path]);
    }
    st.clear_len_resolved(symbolic, destination);
    if place_is_scalar_interval(tcx, local_decls, *destination) {
        st.set_interval_resolved(symbolic, *destination, Interval::top());
    }
    if place_is_float_interval(tcx, local_decls, *destination) {
        st.set_float_interval_resolved(symbolic, *destination, FloatInterval::top());
    }
    let Some(path) = terminator_call_path(tcx, local_decls, term) else {
        return;
    };
    st.debug(format_args!("call {:?} := {path}", destination));
    if let TerminatorKind::Call { func, .. } = &term.kind {
        if let TyKind::FnDef(callee, _) = func.ty(local_decls, tcx).kind() {
            let standard = matches!(
                tcx.crate_name(callee.krate).as_str(),
                "core" | "alloc" | "std"
            ) && !callee.is_local();
            let name = tcx.item_name(*callee);
            if standard {
                if name.as_str() == "from" && args.len() == 1 {
                    if let Some(input) = operand_place(&args[0].node) {
                        if let (TyKind::Adt(src_adt, src_args), TyKind::Adt(dst_adt, dst_args)) = (
                            input.ty(local_decls, tcx).ty.kind(),
                            destination.ty(local_decls, tcx).ty.kind(),
                        ) {
                            let kind = tcx.item_name(src_adt.did());
                            let standard_source = !src_adt.did().is_local()
                                && tcx.crate_name(src_adt.did().krate).as_str() == "core";
                            let standard_target = !dst_adt.did().is_local()
                                && tcx.crate_name(dst_adt.did().krate).as_str() == "core";
                            if standard_source
                                && standard_target
                                && tcx.item_name(dst_adt.did()) == kind
                                && matches!(kind.as_str(), "Range" | "RangeFrom" | "RangeInclusive")
                            {
                                let source = st.clone();
                                let active = src_adt
                                    .non_enum_variant()
                                    .fields
                                    .iter_enumerated()
                                    .find(|(_, field)| field.name.as_str() == "exhausted")
                                    .is_none_or(|(index, field)| {
                                        let field = input.project_deeper(
                                            &[ProjectionElem::Field(
                                                index,
                                                field.ty(tcx, src_args),
                                            )],
                                            tcx,
                                        );
                                        source.tracked_interval_resolved(symbolic, &field)
                                            == Some(Interval::new(0, 0))
                                    });
                                if active {
                                    for (index, field) in
                                        dst_adt.non_enum_variant().fields.iter_enumerated()
                                    {
                                        let output = destination.project_deeper(
                                            &[ProjectionElem::Field(
                                                index,
                                                field.ty(tcx, dst_args),
                                            )],
                                            tcx,
                                        );
                                        if field.name.as_str() == "exhausted" {
                                            st.set_interval_resolved(
                                                symbolic,
                                                output,
                                                Interval::new(0, 0),
                                            );
                                        } else if let Some((input_index, input_field)) = src_adt
                                            .non_enum_variant()
                                            .fields
                                            .iter_enumerated()
                                            .find(|(_, f)| f.name == field.name)
                                        {
                                            let input = input.project_deeper(
                                                &[ProjectionElem::Field(
                                                    input_index,
                                                    input_field.ty(tcx, src_args),
                                                )],
                                                tcx,
                                            );
                                            assign_operand_value(
                                                tcx,
                                                symbolic,
                                                st,
                                                &source,
                                                local_decls,
                                                output,
                                                &Operand::Copy(input),
                                            );
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                if name.as_str() == "len"
                    && args.len() == 1
                    && destination.ty(local_decls, tcx).ty == tcx.types.usize
                {
                    if let Some(receiver) = operand_place(&args[0].node) {
                        let len = st.read_len_resolved_or_top(symbolic, receiver);
                        st.set_interval_resolved(symbolic, *destination, len);
                    }
                }
                if name.as_str() == "new" && args.len() == 2 {
                    if let TyKind::Adt(adt, generics) = destination.ty(local_decls, tcx).ty.kind() {
                        if tcx.item_name(adt.did()).as_str() == "RangeInclusive"
                            && tcx.impl_of_method(*callee).is_some_and(|id| matches!(tcx.type_of(id).instantiate_identity().kind(), TyKind::Adt(owner, _) if owner.did() == adt.did())) {
                            let source = st.clone();
                            for (index, field) in adt.non_enum_variant().fields.iter_enumerated() {
                                let ty = field.ty(tcx, generics);
                                let place = destination.project_deeper(&[ProjectionElem::Field(index, ty)], tcx);
                                match field.name.as_str() {
                                    "start" => assign_operand_value(tcx, symbolic, st, &source, local_decls, place, &args[0].node),
                                    "end" => assign_operand_value(tcx, symbolic, st, &source, local_decls, place, &args[1].node),
                                    "exhausted" => st.set_interval_resolved(symbolic, place, Interval::new(0, 0)),
                                    _ => {},
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    if let TerminatorKind::Call { func, .. } = &term.kind {
        if let TyKind::FnDef(callee, _) = func.ty(local_decls, tcx).kind() {
            if let TyKind::Adt(adt, _) = destination.ty(local_decls, tcx).ty.kind() {
                let is_nonzero = tcx.get_diagnostic_item(rustc_span::Symbol::intern("NonZero"))
                    == Some(adt.did());
                if is_nonzero && tcx.item_name(*callee).as_str() == "new_unchecked"
                    && tcx.impl_of_method(*callee).is_some_and(|id| matches!(tcx.type_of(id).instantiate_identity().kind(), TyKind::Adt(owner, _) if owner.did() == adt.did()))
                {
                    if let (Some(field), Some(arg)) = (scalar_value_place(tcx, *destination, destination.ty(local_decls, tcx).ty), args.first()) {
                        let source = st.clone();
                        assign_operand_value(tcx, symbolic, st, &source, local_decls, field, &arg.node);
                    }
                }
            }
        }
    }

    let ptr_offset = if (path.ends_with("::add")
        || path.ends_with("::wrapping_add")
        || path.ends_with("::offset"))
        && args.len() >= 2
    {
        Some(eval_operand(tcx, local_decls, &args[1].node, symbolic, st))
    } else {
        None
    };
    if let Some(len) = call_len(tcx, local_decls, st, symbolic, term, &path, ptr_offset) {
        st.set_len_resolved(symbolic, *destination, len);
    }
    if let Some(value) = call_interval(tcx, local_decls, term, &path) {
        st.set_interval_resolved(symbolic, *destination, value);
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
    let index = eval_operand(tcx, local_decls, &args[1].node, symbolic, st);
    let Some(index) = singleton_nonnegative(index) else {
        return;
    };
    let Some(len) = operand_slice_or_array_len(tcx, local_decls, st, symbolic, &args[0].node)
        .and_then(singleton_nonnegative)
    else {
        return;
    };
    if index >= len {
        return;
    }
    let mut source_place = receiver_place;
    let mut source_is_ref = true;
    // Array and slice references often carry only a length fact, but they are
    // still needed to resolve the element returned by `get_unchecked`.
    for candidate in st.all_fact_places() {
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
        for candidate in st.all_fact_places() {
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
    if place_is_scalar_interval(tcx, local_decls, dst_deref) {
        let elem_iv = eval_place(symbolic, elem_place, st);
        st.set_interval_resolved(symbolic, dst_deref, elem_iv);
    } else if place_is_float_interval(tcx, local_decls, dst_deref) {
        let elem_iv = eval_float_place(symbolic, elem_place, st);
        st.set_float_interval_resolved(symbolic, dst_deref, elem_iv);
    }
}
