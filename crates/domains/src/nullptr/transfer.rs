use rustc_middle::mir::*;
use rustc_middle::ty::{Ty, TyCtxt, TyKind};

use super::abstract_value::NullPtr;
use super::state::NullPtrState;

fn is_ptr_like(ty: Ty<'_>) -> bool {
    matches!(ty.kind(), TyKind::RawPtr(_, _) | TyKind::FnPtr(..))
}

fn is_ref_like(ty: Ty<'_>) -> bool {
    matches!(ty.kind(), TyKind::Ref(_, _, _))
}

fn is_tracked(ty: Ty<'_>) -> bool {
    is_ptr_like(ty) || is_ref_like(ty)
}

fn get_tracked_value<'tcx>(st: &NullPtrState<'tcx>, place: Place<'tcx>, ty: Ty<'tcx>) -> NullPtr {
    if is_ref_like(ty) {
        st.get_ref(&place)
    } else if is_ptr_like(ty) {
        st.get_nullptr(&place)
    } else {
        NullPtr::Bot
    }
}

fn strip_last_deref<'tcx>(tcx: TyCtxt<'tcx>, place: Place<'tcx>) -> Option<Place<'tcx>> {
    if !matches!(place.projection.last(), Some(ProjectionElem::Deref)) {
        return None;
    }

    let mut base = Place::from(place.local);
    for elem in place
        .projection
        .iter()
        .take(place.projection.len().saturating_sub(1))
    {
        base = base.project_deeper(&[elem.clone()], tcx);
    }
    Some(base)
}

fn base_of_first_deref<'tcx>(tcx: TyCtxt<'tcx>, place: Place<'tcx>) -> Option<Place<'tcx>> {
    let mut base = Place::from(place.local);
    for elem in place.projection.iter() {
        if matches!(elem, ProjectionElem::Deref) {
            return Some(base);
        }
        base = base.project_deeper(&[elem.clone()], tcx);
    }
    None
}

fn set_tracked_value<'tcx>(
    st: &mut NullPtrState<'tcx>,
    place: Place<'tcx>,
    ty: Ty<'tcx>,
    value: NullPtr,
) {
    if is_ref_like(ty) {
        st.set_ref(place, value);
    } else if is_ptr_like(ty) {
        st.set_nullptr(place, value);
    }
}

fn is_null_ctor_path(path: &str) -> bool {
    (path.ends_with("::null") || path.ends_with("::null_mut")) && path.contains("::ptr::")
}

fn bits_to_i128(bits: u128, bit_width: u64, signed: bool) -> i128 {
    if signed {
        if bit_width == 128 {
            bits as i128
        } else {
            let shift = 128 - bit_width;
            ((bits << shift) as i128) >> shift
        }
    } else if bit_width == 128 {
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

fn is_null_const<'tcx>(tcx: TyCtxt<'tcx>, c: &ConstOperand<'tcx>) -> bool {
    let ty = c.ty();
    let Some((bit_width, signed)) = scalar_layout(tcx, ty) else {
        return false;
    };
    let k = c.const_;
    let Some(si) = k.try_to_scalar_int() else {
        return false;
    };
    bits_to_i128(
        si.to_bits_unchecked(),
        bit_width.max(si.size().bits()),
        signed,
    ) == 0
}

pub(crate) fn nullptr_of_const<'tcx>(
    tcx: TyCtxt<'tcx>,
    c: &ConstOperand<'tcx>,
    dst_ty: Ty<'tcx>,
) -> NullPtr {
    if !is_ptr_like(dst_ty) {
        return NullPtr::Bot;
    }

    if is_null_const(tcx, c) {
        NullPtr::Null
    } else {
        NullPtr::NonNull
    }
}

pub(crate) fn eval_operand<'tcx>(
    tcx: TyCtxt<'tcx>,
    local_decls: &'tcx LocalDecls<'tcx>,
    op: &Operand<'tcx>,
    st: &NullPtrState<'tcx>,
    dst_ty: Ty<'tcx>,
) -> NullPtr {
    match op {
        Operand::Copy(p) | Operand::Move(p) => {
            // Rule: a = *b  ==> points[a] = refs[b]
            if let Some(base) = strip_last_deref(tcx, *p) {
                if st.refs.contains_key(&base) {
                    return st.get_ref(&base);
                }
            }

            let src_ty = p.ty(local_decls, tcx).ty;
            get_tracked_value(st, *p, src_ty)
        }
        Operand::Constant(c) => nullptr_of_const(tcx, c, dst_ty),
    }
}

fn eval_cast_nullptr<'tcx>(
    tcx: TyCtxt<'tcx>,
    st: &NullPtrState<'tcx>,
    local_decls: &'tcx LocalDecls<'tcx>,
    op: &Operand<'tcx>,
    dst_ty: Ty<'tcx>,
) -> NullPtr {
    if !is_ptr_like(dst_ty) {
        return NullPtr::Bot;
    }

    let src_ty = op.ty(local_decls, tcx);
    if is_tracked(src_ty) {
        return eval_operand(tcx, local_decls, op, st, dst_ty);
    }
    match op {
        Operand::Constant(c) => {
            if is_null_const(tcx, c) {
                NullPtr::Null
            } else {
                NullPtr::NonNull
            }
        }
        _ => NullPtr::NonNull,
    }
}

fn call_return_value<'tcx>(
    tcx: TyCtxt<'tcx>,
    local_decls: &'tcx LocalDecls<'tcx>,
    func: &Operand<'tcx>,
    dst_ty: Ty<'tcx>,
) -> NullPtr {
    if !is_tracked(dst_ty) {
        return NullPtr::Bot;
    }

    if let TyKind::FnDef(def_id, _) = func.ty(local_decls, tcx).kind() {
        let name = tcx.def_path_str(*def_id);
        if is_null_ctor_path(&name) {
            return NullPtr::Null;
        }
    }

    NullPtr::NonNull
}

pub fn transfer_stmt<'tcx>(
    tcx: TyCtxt<'tcx>,
    st: &mut NullPtrState<'tcx>,
    stmt: &Statement<'tcx>,
    local_decls: &'tcx LocalDecls<'tcx>,
) {
    let kind = &stmt.kind;
    match kind {
        StatementKind::Assign(assign) => {
            let (place, rvalue) = &**assign;
            let dst_ty = place.ty(local_decls, tcx).ty;
            if !is_tracked(dst_ty) {
                return;
            }
            let dst_place = *place;

            let value = match rvalue {
                Rvalue::Use(op) => eval_operand(tcx, local_decls, op, st, dst_ty),
                // Rule: a = &_b  ==> refs[a] = points[b]
                Rvalue::Ref(_, _, borrowed_place) => st.get_nullptr(borrowed_place),
                Rvalue::RawPtr(_, borrowed_place) => {
                    if let Some(base) = base_of_first_deref(tcx, *borrowed_place) {
                        let base_ty = base.ty(local_decls, tcx).ty;
                        if is_ptr_like(base_ty) {
                            st.get_nullptr(&base)
                        } else {
                            NullPtr::NonNull
                        }
                    } else {
                        NullPtr::NonNull
                    }
                }
                Rvalue::CopyForDeref(p) => {
                    let src_ty = p.ty(local_decls, tcx).ty;
                    get_tracked_value(st, *p, src_ty)
                }
                Rvalue::Cast(_, op, cast_ty) => eval_cast_nullptr(tcx, st, local_decls, op, *cast_ty),
                _ => {
                    println!(
                        "Not Support: unhandled tracked rvalue in nullptr analysis: {:?}",
                        rvalue
                    );
                    NullPtr::MaybeNull
                }
            };
            // println!("{:?} = {:?}", dst_place, value);
            set_tracked_value(st, dst_place, dst_ty, value);
        }
        _ => {}
    }
}

pub fn transfer_terminator<'tcx>(
    tcx: TyCtxt<'tcx>,
    st: &mut NullPtrState<'tcx>,
    term: &Terminator<'tcx>,
    local_decls: &'tcx LocalDecls<'tcx>,
) {
    if let TerminatorKind::Call {
        func, destination, ..
    } = &term.kind
    {
        let dst_ty = destination.ty(local_decls, tcx).ty;
        if !is_tracked(dst_ty) {
            return;
        }
        if !(st.pointers.contains_key(destination) || st.refs.contains_key(destination)) {
            return;
        }

        let value = call_return_value(tcx, local_decls, func, dst_ty);
        set_tracked_value(st, *destination, dst_ty, value);
    }
}
