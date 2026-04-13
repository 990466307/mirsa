use rustc_middle::mir::*;
use rustc_middle::ty::{Ty, TyCtxt, TyKind, TypingEnv};

use super::abstract_value::{NullPtr, join};
use super::state::{
    NullPtrState, get_tracked_value, is_ptr_like, is_ref_like, is_tracked, set_tracked_value,
};

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

fn is_null_ctor_path(path: &str) -> bool {
    (path.ends_with("::null") || path.ends_with("::null_mut")) && path.contains("::ptr::")
}

fn unknown_value_for_type(ty: Ty<'_>) -> NullPtr {
    match ty.kind() {
        TyKind::RawPtr(_, _) => NullPtr::MaybeNull,
        TyKind::Ref(_, _, _) | TyKind::FnPtr(..) => NullPtr::NonNull,
        _ => NullPtr::Bot,
    }
}

pub(crate) fn const_nullness<'tcx>(_tcx: TyCtxt<'tcx>, c: &ConstOperand<'tcx>) -> Option<NullPtr> {
    let k = c.const_;

    if let Some(scalar) = k.try_eval_scalar(_tcx, TypingEnv::fully_monomorphized()) {
        return Some(match scalar {
            rustc_middle::mir::interpret::Scalar::Int(i) => {
                if i.is_null() {
                    NullPtr::Null
                } else {
                    NullPtr::NonNull
                }
            }
            rustc_middle::mir::interpret::Scalar::Ptr(_, _) => NullPtr::NonNull,
        });
    }

    if let Some(scalar) = k.try_to_scalar() {
        return Some(match scalar {
            rustc_middle::mir::interpret::Scalar::Int(i) => {
                if i.is_null() {
                    NullPtr::Null
                } else {
                    NullPtr::NonNull
                }
            }
            rustc_middle::mir::interpret::Scalar::Ptr(_, _) => NullPtr::NonNull,
        });
    }

    if let Some(si) = k.try_to_scalar_int() {
        return Some(if si.to_bits_unchecked() == 0 {
            NullPtr::Null
        } else {
            NullPtr::NonNull
        });
    }

    None
}

pub(crate) fn nullptr_of_const<'tcx>(
    tcx: TyCtxt<'tcx>,
    c: &ConstOperand<'tcx>,
    dst_ty: Ty<'tcx>,
) -> NullPtr {
    if !is_ptr_like(dst_ty) {
        return NullPtr::Bot;
    }

    match const_nullness(tcx, c) {
        Some(value) => value,
        None => unknown_value_for_type(dst_ty),
    }
}

fn has_runtime_index<'tcx>(place: Place<'tcx>) -> bool {
    place
        .projection
        .iter()
        .any(|elem| matches!(elem, ProjectionElem::Index(_)))
}

fn candidate_places<'tcx>(
    st: &NullPtrState<'tcx>,
    place: Place<'tcx>,
    ty: Ty<'tcx>,
) -> Vec<Place<'tcx>> {
    let keys: Vec<Place<'tcx>> = if is_ref_like(ty) {
        st.refs.keys().copied().collect()
    } else if is_ptr_like(ty) {
        st.pointers.keys().copied().collect()
    } else {
        Vec::new()
    };

    keys.into_iter()
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
                .all(|(left, right)| match left {
                    ProjectionElem::Index(_) => {
                        matches!(right, ProjectionElem::ConstantIndex { .. })
                    }
                    _ => left == right,
                })
        })
        .collect()
}

fn get_place_value<'tcx>(st: &NullPtrState<'tcx>, place: Place<'tcx>, ty: Ty<'tcx>) -> NullPtr {
    if !has_runtime_index(place) {
        return get_tracked_value(st, place, ty);
    }

    let candidates = candidate_places(st, place, ty);
    if candidates.is_empty() {
        return unknown_value_for_type(ty);
    }

    candidates.into_iter().fold(NullPtr::Bot, |acc, candidate| {
        join(acc, get_tracked_value(st, candidate, ty))
    })
}

fn weak_set_place_value<'tcx>(
    st: &mut NullPtrState<'tcx>,
    place: Place<'tcx>,
    ty: Ty<'tcx>,
    value: NullPtr,
) {
    if !has_runtime_index(place) {
        set_tracked_value(st, place, ty, value);
        return;
    }

    let candidates = candidate_places(st, place, ty);
    if candidates.is_empty() {
        return;
    }

    for candidate in candidates {
        let current = get_tracked_value(st, candidate, ty);
        set_tracked_value(st, candidate, ty, join(current, value));
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
            if let Some(base) = strip_last_deref(tcx, *p) {
                if st.refs.contains_key(&base) {
                    return st.get_ref(&base);
                }
            }

            let src_ty = p.ty(local_decls, tcx).ty;
            get_place_value(st, *p, src_ty)
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
            const_nullness(tcx, c).unwrap_or_else(|| unknown_value_for_type(dst_ty))
        }
        _ => unknown_value_for_type(dst_ty),
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

    unknown_value_for_type(dst_ty)
}

pub fn transfer_stmt<'tcx>(
    tcx: TyCtxt<'tcx>,
    st: &mut NullPtrState<'tcx>,
    stmt: &Statement<'tcx>,
    local_decls: &'tcx LocalDecls<'tcx>,
) {
    let StatementKind::Assign(assign) = &stmt.kind else {
        return;
    };
    let (place, rvalue) = &**assign;
    let dst_ty = place.ty(local_decls, tcx).ty;

    match rvalue {
        Rvalue::Aggregate(kind, operands) => match kind.as_ref() {
            AggregateKind::Tuple => {
                for (i, op) in operands.iter().enumerate() {
                    let field_ty = op.ty(local_decls, tcx);
                    if !is_tracked(field_ty) {
                        continue;
                    }
                    let field_place =
                        place.project_deeper(&[ProjectionElem::Field(i.into(), field_ty)], tcx);
                    let value = eval_operand(tcx, local_decls, op, st, field_ty);
                    set_tracked_value(st, field_place, field_ty, value);
                }
                return;
            }
            AggregateKind::Array(elem_ty) => {
                if !is_tracked(*elem_ty) {
                    return;
                }
                let len = operands.len() as u64;
                for (i, op) in operands.iter().enumerate() {
                    let elem_place = place.project_deeper(
                        &[ProjectionElem::ConstantIndex {
                            offset: i as u64,
                            min_length: len,
                            from_end: false,
                        }],
                        tcx,
                    );
                    let value = eval_operand(tcx, local_decls, op, st, *elem_ty);
                    set_tracked_value(st, elem_place, *elem_ty, value);
                }
                return;
            }
            _ => {}
        },
        _ => {}
    }

    if !is_tracked(dst_ty) {
        return;
    }

    let value = match rvalue {
        Rvalue::Use(op) => eval_operand(tcx, local_decls, op, st, dst_ty),
        Rvalue::Ref(_, _, borrowed_place) => {
            get_place_value(st, *borrowed_place, borrowed_place.ty(local_decls, tcx).ty)
        }
        Rvalue::RawPtr(_, borrowed_place) => {
            if let Some(base) = base_of_first_deref(tcx, *borrowed_place) {
                let base_ty = base.ty(local_decls, tcx).ty;
                if is_ptr_like(base_ty) {
                    get_place_value(st, base, base_ty)
                } else {
                    NullPtr::NonNull
                }
            } else {
                NullPtr::NonNull
            }
        }
        Rvalue::CopyForDeref(p) => {
            let src_ty = p.ty(local_decls, tcx).ty;
            get_place_value(st, *p, src_ty)
        }
        Rvalue::Cast(_, op, cast_ty) => eval_cast_nullptr(tcx, st, local_decls, op, *cast_ty),
        _ => unknown_value_for_type(dst_ty),
    };
    weak_set_place_value(st, *place, dst_ty, value);
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
