use rustc_middle::mir::*;
use rustc_middle::ty::{Ty, TyCtxt, TyKind, TypingEnv};

use super::abstract_value::NullPtr;
use super::state::NullPtrState;
use crate::framework::access_path::AccessPath;

pub(crate) fn is_ptr_like(ty: Ty<'_>) -> bool {
    matches!(ty.kind(), TyKind::RawPtr(_, _) | TyKind::FnPtr(..))
}

fn is_ref_like(ty: Ty<'_>) -> bool {
    matches!(ty.kind(), TyKind::Ref(_, _, _))
}

pub(crate) fn is_tracked(ty: Ty<'_>) -> bool {
    is_ptr_like(ty) || is_ref_like(ty)
}

pub(crate) fn get_tracked_value<'tcx>(
    st: &NullPtrState<'tcx>,
    place: Place<'tcx>,
    ty: Ty<'tcx>,
) -> NullPtr {
    if !is_tracked(ty) {
        return NullPtr::Bot;
    }
    let Some(path) = st.access_path_for_place(place) else {
        return NullPtr::Bot;
    };
    let value = st.value_or_maybe(&path);
    if value == NullPtr::Bot && is_ref_like(ty) {
        NullPtr::NonNull
    } else {
        value
    }
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

fn operand_path<'tcx>(st: &NullPtrState<'tcx>, op: &Operand<'tcx>) -> Option<AccessPath> {
    match op {
        Operand::Copy(place) | Operand::Move(place) => st.access_path_for_place(*place),
        Operand::Constant(_) => None,
    }
}

pub(crate) fn eval_operand<'tcx>(
    tcx: TyCtxt<'tcx>,
    local_decls: &LocalDecls<'tcx>,
    op: &Operand<'tcx>,
    st: &NullPtrState<'tcx>,
    dst_ty: Ty<'tcx>,
) -> NullPtr {
    match op {
        Operand::Copy(place) | Operand::Move(place) => {
            let src_ty = place.ty(local_decls, tcx).ty;
            get_tracked_value(st, *place, src_ty)
        }
        Operand::Constant(c) => {
            if is_ptr_like(dst_ty) {
                const_nullness(tcx, c).unwrap_or_else(|| unknown_value_for_type(dst_ty))
            } else {
                NullPtr::Bot
            }
        }
    }
}

fn assign_place_from_operand<'tcx>(
    tcx: TyCtxt<'tcx>,
    local_decls: &LocalDecls<'tcx>,
    st: &mut NullPtrState<'tcx>,
    dst: Place<'tcx>,
    dst_ty: Ty<'tcx>,
    op: &Operand<'tcx>,
    reason: &str,
) {
    if let Some(src) = operand_path(st, op) {
        st.copy_place_from_path(dst, &src, NullPtr::MaybeNull, reason);
        return;
    }

    let value = eval_operand(tcx, local_decls, op, st, dst_ty);
    st.set_place_path(dst, value);
}

fn first_deref_base<'tcx>(tcx: TyCtxt<'tcx>, place: Place<'tcx>) -> Option<Place<'tcx>> {
    let mut base = Place::from(place.local);
    for elem in place.projection.iter() {
        if matches!(elem, ProjectionElem::Deref) {
            return Some(base);
        }
        base = base.project_deeper(&[elem.clone()], tcx);
    }
    None
}

pub fn transfer_stmt<'tcx>(
    tcx: TyCtxt<'tcx>,
    st: &mut NullPtrState<'tcx>,
    stmt: &Statement<'tcx>,
    local_decls: &LocalDecls<'tcx>,
) {
    st.debug(format_args!("mir stmt: {:?}", stmt.kind));

    let StatementKind::Assign(assign) = &stmt.kind else {
        return;
    };
    let (place, rvalue) = &**assign;
    let dst_ty = place.ty(local_decls, tcx).ty;
    let Some(dst_path) = st.access_path_for_place(*place) else {
        return;
    };

    if is_tracked(dst_ty) || matches!(rvalue, Rvalue::Aggregate(..)) {
        st.debug(format_args!("assign {:?} = {:?}", place, rvalue));
    }

    match rvalue {
        Rvalue::Aggregate(kind, operands) => match kind.as_ref() {
            AggregateKind::Tuple => {
                for (idx, op) in operands.iter().enumerate() {
                    let field_ty = op.ty(local_decls, tcx);
                    let field_place =
                        place.project_deeper(&[ProjectionElem::Field(idx.into(), field_ty)], tcx);
                    if let Some(src) = operand_path(st, op) {
                        if is_tracked(field_ty) {
                            st.copy_place_from_path(
                                field_place,
                                &src,
                                NullPtr::MaybeNull,
                                "aggregate",
                            );
                        } else if let Some(dst) = st.access_path_for_place(field_place) {
                            st.copy_child_subtree(&dst, &src, NullPtr::MaybeNull, "aggregate");
                        }
                    } else if is_tracked(field_ty) {
                        assign_place_from_operand(
                            tcx,
                            local_decls,
                            st,
                            field_place,
                            field_ty,
                            op,
                            "aggregate",
                        );
                    }
                }
                return;
            }
            AggregateKind::Array(elem_ty) => {
                if !is_tracked(*elem_ty) {
                    return;
                }
                let len = operands.len() as u64;
                for (idx, op) in operands.iter().enumerate() {
                    let elem_place = place.project_deeper(
                        &[ProjectionElem::ConstantIndex {
                            offset: idx as u64,
                            min_length: len,
                            from_end: false,
                        }],
                        tcx,
                    );
                    if let Some(src) = operand_path(st, op) {
                        st.copy_place_from_path(elem_place, &src, NullPtr::MaybeNull, "aggregate");
                    } else {
                        assign_place_from_operand(
                            tcx,
                            local_decls,
                            st,
                            elem_place,
                            *elem_ty,
                            op,
                            "aggregate",
                        );
                    }
                }
                return;
            }
            _ => {}
        },
        _ => {}
    }

    if !is_tracked(dst_ty) {
        if let Rvalue::Use(Operand::Copy(src) | Operand::Move(src)) = rvalue {
            if let Some(src_path) = st.access_path_for_place(*src) {
                st.copy_child_subtree(&dst_path, &src_path, NullPtr::MaybeNull, "assign");
            }
        }
        return;
    }

    match rvalue {
        Rvalue::Use(op) => {
            assign_place_from_operand(tcx, local_decls, st, *place, dst_ty, op, "assign");
        }
        Rvalue::CopyForDeref(src) => {
            if let Some(src_path) = st.access_path_for_place(*src) {
                st.copy_subtree(&dst_path, &src_path, NullPtr::MaybeNull, "load");
            } else {
                st.set_place_path(*place, unknown_value_for_type(dst_ty));
            }
        }
        Rvalue::Ref(_, _, borrowed_place) => {
            if let Some(src_path) = st.access_path_for_place(*borrowed_place) {
                st.copy_subtree(
                    &dst_path.deref(),
                    &src_path,
                    NullPtr::MaybeNull,
                    "ref-pointee",
                );
            }
            st.set_path(dst_path, NullPtr::NonNull);
        }
        Rvalue::RawPtr(_, borrowed_place) => {
            if let Some(src_path) = st.access_path_for_place(*borrowed_place) {
                st.copy_subtree(
                    &dst_path.deref(),
                    &src_path,
                    NullPtr::MaybeNull,
                    "raw-pointee",
                );
            }
            let value = if let Some(base) = first_deref_base(tcx, *borrowed_place) {
                let base_ty = base.ty(local_decls, tcx).ty;
                if is_ptr_like(base_ty) {
                    get_tracked_value(st, base, base_ty)
                } else {
                    NullPtr::NonNull
                }
            } else {
                NullPtr::NonNull
            };
            st.set_path(dst_path, value);
        }
        Rvalue::Cast(_, op, cast_ty) => {
            if !is_ptr_like(*cast_ty) {
                st.set_place_path(*place, NullPtr::Bot);
            } else {
                assign_place_from_operand(tcx, local_decls, st, *place, *cast_ty, op, "cast");
            }
        }
        _ => st.set_place_path(*place, unknown_value_for_type(dst_ty)),
    }
}

pub fn transfer_terminator<'tcx>(
    tcx: TyCtxt<'tcx>,
    st: &mut NullPtrState<'tcx>,
    term: &Terminator<'tcx>,
    local_decls: &LocalDecls<'tcx>,
) {
    st.debug(format_args!("mir terminator: {:?}", term.kind));

    if let TerminatorKind::Call {
        func,
        args,
        destination,
        ..
    } = &term.kind
    {
        let dst_ty = destination.ty(local_decls, tcx).ty;
        if is_tracked(dst_ty) && st.contains_place(*destination) {
            if let Some(dst_path) = st.access_path_for_place(*destination) {
                let mut handled = false;
                if let TyKind::FnDef(def_id, _) = func.ty(local_decls, tcx).kind() {
                    let name = tcx.def_path_str(*def_id);
                    st.debug(format_args!("call {:?} := {name}", destination));
                    if (name.ends_with("::null") || name.ends_with("::null_mut"))
                        && name.contains("::ptr::")
                    {
                        st.set_path(dst_path.clone(), NullPtr::Null);
                        handled = true;
                    } else if name.ends_with("::cast")
                        || name.ends_with("::cast_const")
                        || name.ends_with("::cast_mut")
                        || name.ends_with("::with_addr")
                        || name.ends_with("::map_addr")
                    {
                        if let Some(first) = args.first() {
                            assign_place_from_operand(
                                tcx,
                                local_decls,
                                st,
                                *destination,
                                dst_ty,
                                &first.node,
                                "call-cast",
                            );
                            handled = true;
                        }
                    }
                }

                if !handled {
                    st.set_path(dst_path, unknown_value_for_type(dst_ty));
                }
            }
        }
    }

    st.debug_map("bb end map");
}
