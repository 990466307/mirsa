use rustc_middle::mir::*;
use rustc_middle::ty::{Ty, TyCtxt, TyKind, TypingEnv};

use super::abstract_value::NullPtr;
use super::state::NullPtrState;
use mirsa_framework::access_path::AccessPath;
use mirsa_relations::symbolic::SymbolicState;

pub(crate) fn is_ptr_like(ty: Ty<'_>) -> bool {
    matches!(ty.kind(), TyKind::RawPtr(_, _) | TyKind::FnPtr(..))
}

fn is_ref_like(ty: Ty<'_>) -> bool {
    matches!(ty.kind(), TyKind::Ref(_, _, _))
}

pub fn is_tracked(ty: Ty<'_>) -> bool {
    is_ptr_like(ty) || is_ref_like(ty)
}

pub fn get_tracked_value<'tcx>(
    st: &mut NullPtrState<'tcx>,
    symbolic: &SymbolicState<'tcx>,
    place: Place<'tcx>,
    ty: Ty<'tcx>,
) -> NullPtr {
    if !is_tracked(ty) {
        return NullPtr::Bot;
    }
    let Some(path) = st.access_path_for_place_resolved(symbolic, place) else {
        return NullPtr::MaybeNull;
    };
    let value = st.value_or_maybe(&path);
    st.set_place_path_resolved(symbolic, place, value);
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

pub fn const_nullness<'tcx>(_tcx: TyCtxt<'tcx>, c: &ConstOperand<'tcx>) -> Option<NullPtr> {
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

fn operand_path<'tcx>(
    st: &NullPtrState<'tcx>,
    symbolic: &SymbolicState<'tcx>,
    op: &Operand<'tcx>,
) -> Option<AccessPath> {
    match op {
        Operand::Copy(place) | Operand::Move(place) => {
            st.access_path_for_place_resolved(symbolic, *place)
        }
        Operand::Constant(_) => None,
    }
}

fn has_runtime_index<'tcx>(place: Place<'tcx>) -> bool {
    place
        .projection
        .iter()
        .any(|elem| matches!(elem, ProjectionElem::Index(_)))
}

pub fn eval_operand<'tcx>(
    tcx: TyCtxt<'tcx>,
    local_decls: &LocalDecls<'tcx>,
    op: &Operand<'tcx>,
    st: &mut NullPtrState<'tcx>,
    symbolic: &SymbolicState<'tcx>,
    dst_ty: Ty<'tcx>,
) -> NullPtr {
    match op {
        Operand::Copy(place) | Operand::Move(place) => {
            let src_ty = place.ty(local_decls, tcx).ty;
            if is_tracked(src_ty) {
                get_tracked_value(st, symbolic, *place, src_ty)
            } else {
                unknown_value_for_type(dst_ty)
            }
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
    symbolic: &SymbolicState<'tcx>,
    dst: Place<'tcx>,
    dst_ty: Ty<'tcx>,
    op: &Operand<'tcx>,
    reason: &str,
) {
    if let Operand::Copy(place) | Operand::Move(place) = op {
        if is_tracked(place.ty(local_decls, tcx).ty) {
            if let Some(src) = operand_path(st, symbolic, op) {
                st.copy_place_from_path_resolved(symbolic, dst, &src, NullPtr::MaybeNull, reason);
                return;
            }
        }
    }

    let value = eval_operand(tcx, local_decls, op, st, symbolic, dst_ty);
    st.set_place_path_resolved(symbolic, dst, value);
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
    symbolic: &SymbolicState<'tcx>,
    st: &mut NullPtrState<'tcx>,
    stmt: &Statement<'tcx>,
    local_decls: &LocalDecls<'tcx>,
) {
    let StatementKind::Assign(assign) = &stmt.kind else {
        return;
    };
    let (place, rvalue) = &**assign;
    let dst_ty = place.ty(local_decls, tcx).ty;
    let Some(dst_path) = st.access_path_for_place_resolved(symbolic, *place) else {
        return;
    };

    if is_tracked(dst_ty) || matches!(rvalue, Rvalue::Aggregate(..)) {
        st.debug(format_args!("stmt assign {:?} = {:?}", place, rvalue));
    }

    if has_runtime_index(*place) {
        return;
    }

    if let Rvalue::Use(Operand::Copy(src) | Operand::Move(src)) = rvalue {
        if has_runtime_index(*src) && !has_runtime_index(*place) && is_tracked(dst_ty) {
            return;
        }
    }

    match rvalue {
        Rvalue::Aggregate(kind, operands) => match kind.as_ref() {
            AggregateKind::Tuple => {
                for (idx, op) in operands.iter().enumerate() {
                    let field_ty = op.ty(local_decls, tcx);
                    let field_place =
                        place.project_deeper(&[ProjectionElem::Field(idx.into(), field_ty)], tcx);
                    if let Some(src) = operand_path(st, symbolic, op) {
                        if is_tracked(field_ty) {
                            st.copy_place_from_path_resolved(
                                symbolic,
                                field_place,
                                &src,
                                NullPtr::MaybeNull,
                                "aggregate",
                            );
                        } else if let Some(dst) =
                            st.access_path_for_place_resolved(symbolic, field_place)
                        {
                            st.copy_child_subtree(&dst, &src, NullPtr::MaybeNull, "aggregate");
                        }
                    } else if is_tracked(field_ty) {
                        assign_place_from_operand(
                            tcx,
                            local_decls,
                            st,
                            symbolic,
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
                    if let Some(src) = operand_path(st, symbolic, op) {
                        st.copy_place_from_path_resolved(
                            symbolic,
                            elem_place,
                            &src,
                            NullPtr::MaybeNull,
                            "aggregate",
                        );
                    } else {
                        assign_place_from_operand(
                            tcx,
                            local_decls,
                            st,
                            symbolic,
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
            if let Some(src_path) = st.access_path_for_place_resolved(symbolic, *src) {
                st.copy_child_subtree(&dst_path, &src_path, NullPtr::MaybeNull, "assign");
            }
        }
        return;
    }

    match rvalue {
        Rvalue::Use(op) => {
            assign_place_from_operand(tcx, local_decls, st, symbolic, *place, dst_ty, op, "assign");
        }
        Rvalue::CopyForDeref(src) => {
            if let Some(src_path) = st.access_path_for_place_resolved(symbolic, *src) {
                st.copy_subtree(&dst_path, &src_path, NullPtr::MaybeNull, "load");
            } else {
                st.set_place_path_resolved(symbolic, *place, unknown_value_for_type(dst_ty));
            }
        }
        Rvalue::Ref(_, _, _) => {
            st.set_path(dst_path, NullPtr::NonNull);
        }
        Rvalue::RawPtr(_, borrowed_place) => {
            let value = if let Some(base) = first_deref_base(tcx, *borrowed_place) {
                let base_ty = base.ty(local_decls, tcx).ty;
                if is_ptr_like(base_ty) {
                    get_tracked_value(st, symbolic, base, base_ty)
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
                st.set_place_path_resolved(symbolic, *place, NullPtr::Bot);
            } else {
                assign_place_from_operand(
                    tcx,
                    local_decls,
                    st,
                    symbolic,
                    *place,
                    *cast_ty,
                    op,
                    "cast",
                );
            }
        }
        _ => st.set_place_path_resolved(symbolic, *place, unknown_value_for_type(dst_ty)),
    }
}

pub fn transfer_terminator<'tcx>(
    tcx: TyCtxt<'tcx>,
    symbolic: &SymbolicState<'tcx>,
    st: &mut NullPtrState<'tcx>,
    term: &Terminator<'tcx>,
    local_decls: &LocalDecls<'tcx>,
) {
    if let TerminatorKind::Call {
        func,
        args,
        destination,
        ..
    } = &term.kind
    {
        let dst_ty = destination.ty(local_decls, tcx).ty;
        if is_tracked(dst_ty) {
            if let Some(dst_path) = st.access_path_for_place_resolved(symbolic, *destination) {
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
                                symbolic,
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
}
