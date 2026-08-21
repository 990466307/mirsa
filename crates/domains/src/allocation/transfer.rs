use super::abstract_value::{AllocationId, AllocationSite, LayoutValue, PointerValue};
use super::state::{
    AllocationState, is_allocation_pointer, is_layout_ty, is_non_null_ty, pointer_pointee_type,
    type_size,
};
use crate::interval::abstract_value::{Interval, div, mul, neg, sub};
use mirsa_framework::access_path::AccessPath;
use mirsa_relations::symbolic::SymbolicState;
use rustc_middle::mir::*;
use rustc_middle::ty::{Ty, TyCtxt, TyKind, TypingEnv};
use rustc_span::source_map::Spanned;

pub fn transfer_stmt<'tcx>(
    tcx: TyCtxt<'tcx>,
    symbolic: &SymbolicState<'tcx>,
    state: &mut AllocationState<'tcx>,
    stmt: &Statement<'tcx>,
    local_decls: &LocalDecls<'tcx>,
    mut integer_value: impl FnMut(&Operand<'tcx>) -> Interval,
) {
    match &stmt.kind {
        StatementKind::StorageLive(local) => {
            state.set_stack_live(*local, true);
            return;
        }
        StatementKind::StorageDead(local) => {
            state.set_stack_live(*local, false);
            return;
        }
        StatementKind::Assign(assign) => {
            let (destination, rvalue) = &**assign;
            transfer_assign(
                tcx,
                symbolic,
                state,
                *destination,
                rvalue,
                local_decls,
                &mut integer_value,
            );
        }
        _ => {}
    }
}

fn transfer_assign<'tcx>(
    tcx: TyCtxt<'tcx>,
    symbolic: &SymbolicState<'tcx>,
    state: &mut AllocationState<'tcx>,
    destination: Place<'tcx>,
    rvalue: &Rvalue<'tcx>,
    local_decls: &LocalDecls<'tcx>,
    integer_value: &mut impl FnMut(&Operand<'tcx>) -> Interval,
) {
    if has_runtime_index(destination) {
        return;
    }
    if let Rvalue::Use(Operand::Copy(source) | Operand::Move(source)) = rvalue {
        if has_runtime_index(*source) {
            return;
        }
    }

    let destination_ty = destination.ty(local_decls, tcx).ty;
    let destination_path = state.pointer_path_resolved(symbolic, destination);

    if is_layout_ty(tcx, destination_ty) {
        let value = match rvalue {
            Rvalue::Use(Operand::Copy(source) | Operand::Move(source)) => {
                state.layout_value_resolved(symbolic, *source)
            }
            _ => LayoutValue::top(),
        };
        state.set_layout_resolved(symbolic, destination, value);
    }

    if let Rvalue::Aggregate(kind, operands) = rvalue {
        let operands: Vec<_> = operands.iter().cloned().collect();
        transfer_aggregate(
            tcx,
            symbolic,
            state,
            destination,
            kind,
            &operands,
            local_decls,
        );
        return;
    }

    if !is_allocation_pointer(tcx, destination_ty) {
        if let Rvalue::Use(Operand::Copy(source) | Operand::Move(source)) = rvalue {
            if let (Some(destination_path), Some(source_path)) = (
                destination_path,
                state.pointer_path_resolved(symbolic, *source),
            ) {
                state.copy_pointer_tree(&destination_path, &source_path, PointerValue::top());
            }
        }
        return;
    }

    let value = eval_pointer_rvalue(
        tcx,
        symbolic,
        state,
        rvalue,
        local_decls,
        destination_ty,
        integer_value,
    );
    state.set_pointer_resolved(symbolic, destination, value);
}

pub fn eval_pointer_rvalue<'tcx>(
    tcx: TyCtxt<'tcx>,
    symbolic: &SymbolicState<'tcx>,
    state: &AllocationState<'tcx>,
    rvalue: &Rvalue<'tcx>,
    local_decls: &LocalDecls<'tcx>,
    destination_ty: Ty<'tcx>,
    integer_value: &mut impl FnMut(&Operand<'tcx>) -> Interval,
) -> PointerValue {
    match rvalue {
        Rvalue::Use(operand) => {
            eval_pointer_operand(tcx, symbolic, state, operand, local_decls, destination_ty)
        }
        Rvalue::CopyForDeref(source) => state.pointer_value_resolved(symbolic, *source),
        Rvalue::Ref(_, _, borrowed) | Rvalue::RawPtr(_, borrowed) => {
            address_of_place(tcx, symbolic, state, *borrowed, local_decls, integer_value)
        }
        Rvalue::Cast(_, operand, cast_ty) if is_allocation_pointer(tcx, *cast_ty) => {
            let source_ty = operand.ty(local_decls, tcx);
            if is_allocation_pointer(tcx, source_ty) {
                eval_pointer_operand(tcx, symbolic, state, operand, local_decls, destination_ty)
            } else {
                let value = integer_value(operand);
                if value.low == 0 && value.high == 0 {
                    PointerValue::null()
                } else {
                    PointerValue::top()
                }
            }
        }
        Rvalue::BinaryOp(BinOp::Offset, operands) => {
            let (base, count) = &**operands;
            let base_value = eval_pointer_operand(
                tcx,
                symbolic,
                state,
                base,
                local_decls,
                base.ty(local_decls, tcx),
            );
            let scale = pointer_element_size(tcx, base.ty(local_decls, tcx));
            base_value.add_offset(mul(&integer_value(count), &scale))
        }
        _ => PointerValue::top(),
    }
}

fn has_runtime_index(place: Place<'_>) -> bool {
    place
        .projection
        .iter()
        .any(|elem| matches!(elem, ProjectionElem::Index(_)))
}

fn transfer_aggregate<'tcx>(
    tcx: TyCtxt<'tcx>,
    symbolic: &SymbolicState<'tcx>,
    state: &mut AllocationState<'tcx>,
    destination: Place<'tcx>,
    kind: &Box<AggregateKind<'tcx>>,
    operands: &[Operand<'tcx>],
    local_decls: &LocalDecls<'tcx>,
) {
    match kind.as_ref() {
        AggregateKind::Tuple => {
            for (index, operand) in operands.iter().enumerate() {
                let field_ty = operand.ty(local_decls, tcx);
                let field = destination
                    .project_deeper(&[ProjectionElem::Field(index.into(), field_ty)], tcx);
                transfer_aggregate_operand(tcx, symbolic, state, field, operand, local_decls);
            }
        }
        AggregateKind::Array(element_ty) => {
            let len = operands.len() as u64;
            for (index, operand) in operands.iter().enumerate() {
                let element = destination.project_deeper(
                    &[ProjectionElem::ConstantIndex {
                        offset: index as u64,
                        min_length: len,
                        from_end: false,
                    }],
                    tcx,
                );
                if is_allocation_pointer(tcx, *element_ty) {
                    transfer_aggregate_operand(tcx, symbolic, state, element, operand, local_decls);
                }
            }
        }
        _ => {
            // ADT field paths are copied by suffix when the operand itself is
            // an aggregate carrying tracked pointer fields.
            for operand in operands {
                let (Operand::Copy(source) | Operand::Move(source)) = operand else {
                    continue;
                };
                let (Some(dst), Some(src)) = (
                    state.pointer_path_resolved(symbolic, destination),
                    state.pointer_path_resolved(symbolic, *source),
                ) else {
                    continue;
                };
                state.copy_pointer_tree(&dst, &src, PointerValue::top());
            }
        }
    }
}

fn transfer_aggregate_operand<'tcx>(
    tcx: TyCtxt<'tcx>,
    symbolic: &SymbolicState<'tcx>,
    state: &mut AllocationState<'tcx>,
    destination: Place<'tcx>,
    operand: &Operand<'tcx>,
    local_decls: &LocalDecls<'tcx>,
) {
    let ty = destination.ty(local_decls, tcx).ty;
    if is_allocation_pointer(tcx, ty) {
        let value = eval_pointer_operand(tcx, symbolic, state, operand, local_decls, ty);
        state.set_pointer_resolved(symbolic, destination, value);
    } else if let Operand::Copy(source) | Operand::Move(source) = operand {
        if let (Some(dst), Some(src)) = (
            state.pointer_path_resolved(symbolic, destination),
            state.pointer_path_resolved(symbolic, *source),
        ) {
            state.copy_pointer_tree(&dst, &src, PointerValue::top());
        }
    }
}

pub fn transfer_terminator<'tcx>(
    tcx: TyCtxt<'tcx>,
    symbolic: &SymbolicState<'tcx>,
    state: &mut AllocationState<'tcx>,
    term: &Terminator<'tcx>,
    local_decls: &LocalDecls<'tcx>,
    mut integer_value: impl FnMut(&Operand<'tcx>) -> Interval,
) {
    let TerminatorKind::Call {
        func,
        args,
        destination,
        ..
    } = &term.kind
    else {
        return;
    };
    let destination_ty = destination.ty(local_decls, tcx).ty;
    let Some((def_id, generic_args)) = call_def(tcx, local_decls, func) else {
        if is_allocation_pointer(tcx, destination_ty) {
            state.set_pointer_resolved(symbolic, *destination, PointerValue::top());
        }
        if is_layout_ty(tcx, destination_ty) {
            state.set_layout_resolved(symbolic, *destination, LayoutValue::top());
        }
        return;
    };
    let path = tcx.def_path_str(def_id);
    state.debug(format_args!("call {destination:?} := {path}"));

    if is_layout_ty(tcx, destination_ty) {
        let layout = layout_constructor_value(
            tcx,
            state,
            symbolic,
            local_decls,
            &path,
            generic_args,
            args,
            &mut integer_value,
        )
        .unwrap_or_else(LayoutValue::top);
        state.set_layout_resolved(symbolic, *destination, layout);
    }

    if is_deallocate_call(&path) {
        if let Some(pointer) = first_deallocation_pointer_argument(tcx, local_decls, args) {
            let value = eval_pointer_operand(
                tcx,
                symbolic,
                state,
                pointer,
                local_decls,
                pointer.ty(local_decls, tcx),
            );
            state.deallocate(&value, false);
        }
        return;
    }

    if is_reallocate_call(&path) && is_allocation_pointer(tcx, destination_ty) {
        if let Some(pointer) = first_deallocation_pointer_argument(tcx, local_decls, args) {
            let old = eval_pointer_operand(
                tcx,
                symbolic,
                state,
                pointer,
                local_decls,
                pointer.ty(local_decls, tcx),
            );
            state.deallocate(&old, true);
        }
        let extent = args
            .iter()
            .rev()
            .find(|arg| is_integer(arg.node.ty(local_decls, tcx)))
            .map(|arg| integer_value(&arg.node))
            .unwrap_or_else(Interval::top);
        let id = heap_id(term, symbolic, state, *destination);
        let result = state.allocate_fallibly(id, extent);
        state.set_pointer_resolved(symbolic, *destination, result);
        return;
    }

    if is_allocate_call(&path) && is_allocation_pointer(tcx, destination_ty) {
        let extent = args
            .iter()
            .rev()
            .find_map(|arg| {
                is_layout_ty(tcx, arg.node.ty(local_decls, tcx))
                    .then(|| eval_layout_operand(tcx, state, symbolic, local_decls, &arg.node).size)
            })
            .unwrap_or_else(Interval::top);
        let id = heap_id(term, symbolic, state, *destination);
        let result = state.allocate_fallibly(id, extent);
        state.set_pointer_resolved(symbolic, *destination, result);
        return;
    }

    if !is_allocation_pointer(tcx, destination_ty) {
        return;
    }

    let value = pointer_call_value(
        tcx,
        symbolic,
        state,
        local_decls,
        &path,
        args,
        destination_ty,
        &mut integer_value,
    );
    state.set_pointer_resolved(symbolic, *destination, value);
}

fn pointer_call_value<'tcx>(
    tcx: TyCtxt<'tcx>,
    symbolic: &SymbolicState<'tcx>,
    state: &AllocationState<'tcx>,
    local_decls: &LocalDecls<'tcx>,
    path: &str,
    args: &[Spanned<Operand<'tcx>>],
    destination_ty: Ty<'tcx>,
    integer_value: &mut impl FnMut(&Operand<'tcx>) -> Interval,
) -> PointerValue {
    if (path.ends_with("::null") || path.ends_with("::null_mut")) && path.contains("::ptr::") {
        return PointerValue::null();
    }

    if path.ends_with("::dangling") && path.contains("::NonNull") {
        return PointerValue::invalid_non_null();
    }

    let Some(base_index) = args
        .iter()
        .position(|arg| is_allocation_pointer(tcx, arg.node.ty(local_decls, tcx)))
    else {
        return PointerValue::top();
    };
    let base_operand = &args[base_index].node;
    let base = eval_pointer_operand(
        tcx,
        symbolic,
        state,
        base_operand,
        local_decls,
        destination_ty,
    );

    if is_pointer_offset_call(path) {
        let Some(offset_operand) = args.get(base_index + 1).map(|arg| &arg.node) else {
            return base.forget_offsets();
        };
        let mut delta = integer_value(offset_operand);
        if !is_byte_offset_call(path) {
            delta = mul(
                &delta,
                &pointer_element_size(tcx, base_operand.ty(local_decls, tcx)),
            );
        }
        if is_subtracting_offset_call(path) {
            delta = neg(&delta);
        }
        return base.add_offset(delta);
    }

    if path.ends_with("::with_addr") || path.ends_with("::map_addr") || path.ends_with("::mask") {
        return base.forget_offsets();
    }

    if path.ends_with("::cast")
        || path.ends_with("::cast_const")
        || path.ends_with("::cast_mut")
        || path.ends_with("::as_ptr")
        || path.ends_with("::as_mut_ptr")
        || path.ends_with("::as_ref")
        || path.ends_with("::as_mut")
        || path.ends_with("::new_unchecked")
        || path.ends_with("::slice_from_raw_parts")
        || path.ends_with("::from")
    {
        return base;
    }

    PointerValue::top()
}

pub fn layout_scalar_call_result<'tcx>(
    tcx: TyCtxt<'tcx>,
    symbolic: &SymbolicState<'tcx>,
    state: &AllocationState<'tcx>,
    term: &Terminator<'tcx>,
    local_decls: &LocalDecls<'tcx>,
) -> Option<Interval> {
    let TerminatorKind::Call { func, args, .. } = &term.kind else {
        return None;
    };
    let (def_id, _) = call_def(tcx, local_decls, func)?;
    let path = tcx.def_path_str(def_id);
    let layout = args
        .iter()
        .find(|arg| is_layout_operand_ty(tcx, local_decls, &arg.node))
        .map(|arg| eval_layout_operand(tcx, state, symbolic, local_decls, &arg.node))?;
    if path.ends_with("::Layout::size") {
        Some(layout.size)
    } else if path.ends_with("::Layout::align") {
        Some(layout.align)
    } else {
        None
    }
}

pub fn pointer_difference_call_result<'tcx>(
    tcx: TyCtxt<'tcx>,
    symbolic: &SymbolicState<'tcx>,
    state: &AllocationState<'tcx>,
    term: &Terminator<'tcx>,
    local_decls: &LocalDecls<'tcx>,
) -> Option<Interval> {
    let TerminatorKind::Call { func, args, .. } = &term.kind else {
        return None;
    };
    let (def_id, _) = call_def(tcx, local_decls, func)?;
    let path = tcx.def_path_str(def_id);
    if !is_pointer_difference_call(&path) {
        return None;
    }
    let pointers: Vec<_> = args
        .iter()
        .filter(|arg| is_allocation_pointer(tcx, arg.node.ty(local_decls, tcx)))
        .map(|arg| &arg.node)
        .take(2)
        .collect();
    let [left, right] = pointers.as_slice() else {
        return None;
    };
    let left_value = eval_pointer_operand(
        tcx,
        symbolic,
        state,
        left,
        local_decls,
        left.ty(local_decls, tcx),
    );
    let right_value = eval_pointer_operand(
        tcx,
        symbolic,
        state,
        right,
        local_decls,
        right.ty(local_decls, tcx),
    );
    let (left_id, left_offset) = left_value.exact_target()?;
    let (right_id, right_offset) = right_value.exact_target()?;
    if left_id != right_id
        || !state
            .object(left_id)
            .is_some_and(|fact| fact.multiplicity.is_exactly_one())
    {
        return None;
    }
    let bytes = sub(&left_offset, &right_offset);
    if is_byte_difference_call(&path) {
        Some(bytes)
    } else {
        let element_size = pointer_element_size(tcx, left.ty(local_decls, tcx));
        Some(div(&bytes, &element_size))
    }
}

fn layout_constructor_value<'tcx>(
    tcx: TyCtxt<'tcx>,
    state: &AllocationState<'tcx>,
    symbolic: &SymbolicState<'tcx>,
    local_decls: &LocalDecls<'tcx>,
    path: &str,
    generic_args: rustc_middle::ty::GenericArgsRef<'tcx>,
    args: &[Spanned<Operand<'tcx>>],
    integer_value: &mut impl FnMut(&Operand<'tcx>) -> Interval,
) -> Option<LayoutValue> {
    if path.ends_with("::Layout::from_size_align_unchecked") && args.len() >= 2 {
        return Some(LayoutValue::new(
            integer_value(&args[0].node),
            integer_value(&args[1].node),
        ));
    }
    if path.ends_with("::Layout::new") {
        let ty = generic_args.types().next()?;
        let layout = tcx
            .layout_of(TypingEnv::fully_monomorphized().as_query_input(ty))
            .ok()?;
        let size = layout.size.bytes() as i128;
        let align = layout.align.abi.bytes() as i128;
        return Some(LayoutValue::new(
            Interval::new(size, size),
            Interval::new(align, align),
        ));
    }
    if let Some(first) = args.first() {
        if is_layout_operand_ty(tcx, local_decls, &first.node) {
            return Some(eval_layout_operand(
                tcx,
                state,
                symbolic,
                local_decls,
                &first.node,
            ));
        }
    }
    None
}

fn eval_layout_operand<'tcx>(
    tcx: TyCtxt<'tcx>,
    state: &AllocationState<'tcx>,
    symbolic: &SymbolicState<'tcx>,
    local_decls: &LocalDecls<'tcx>,
    operand: &Operand<'tcx>,
) -> LayoutValue {
    match operand {
        Operand::Copy(place) | Operand::Move(place) => match place.ty(local_decls, tcx).ty.kind() {
            TyKind::Ref(_, inner, _) if is_layout_ty(tcx, *inner) => {
                let Some(path) = AccessPath::from_place(*place) else {
                    return LayoutValue::top();
                };
                state.layout_value(&symbolic.normalize_path(&path.deref()))
            }
            _ => state.layout_value_resolved(symbolic, *place),
        },
        Operand::Constant(_) => LayoutValue::top(),
    }
}

fn is_layout_operand_ty<'tcx>(
    tcx: TyCtxt<'tcx>,
    local_decls: &LocalDecls<'tcx>,
    operand: &Operand<'tcx>,
) -> bool {
    match operand.ty(local_decls, tcx).kind() {
        TyKind::Ref(_, inner, _) => is_layout_ty(tcx, *inner),
        _ => is_layout_ty(tcx, operand.ty(local_decls, tcx)),
    }
}

pub fn eval_pointer_operand<'tcx>(
    tcx: TyCtxt<'tcx>,
    symbolic: &SymbolicState<'tcx>,
    state: &AllocationState<'tcx>,
    operand: &Operand<'tcx>,
    local_decls: &LocalDecls<'tcx>,
    destination_ty: Ty<'tcx>,
) -> PointerValue {
    match operand {
        Operand::Copy(place) | Operand::Move(place) => {
            if is_allocation_pointer(tcx, place.ty(local_decls, tcx).ty) {
                state.pointer_value_resolved(symbolic, *place)
            } else if is_allocation_pointer(tcx, destination_ty) {
                PointerValue::top()
            } else {
                PointerValue::bottom()
            }
        }
        Operand::Constant(constant) => {
            if !is_allocation_pointer(tcx, destination_ty) {
                return PointerValue::bottom();
            }
            if constant
                .const_
                .try_to_scalar_int()
                .is_some_and(|value| value.to_bits_unchecked() == 0)
            {
                PointerValue::null()
            } else {
                PointerValue::top()
            }
        }
    }
}

fn address_of_place<'tcx>(
    tcx: TyCtxt<'tcx>,
    symbolic: &SymbolicState<'tcx>,
    state: &AllocationState<'tcx>,
    place: Place<'tcx>,
    local_decls: &LocalDecls<'tcx>,
    integer_value: &mut impl FnMut(&Operand<'tcx>) -> Interval,
) -> PointerValue {
    let first_deref = place
        .projection
        .iter()
        .position(|elem| matches!(elem, ProjectionElem::Deref));
    let (base, start_projection, mut value) = if let Some(deref_index) = first_deref {
        let base = Place::from(place.local).project_deeper(&place.projection[..deref_index], tcx);
        (
            base.project_deeper(&[ProjectionElem::Deref], tcx),
            deref_index + 1,
            state.pointer_value_resolved(symbolic, base),
        )
    } else {
        (
            Place::from(place.local),
            0,
            PointerValue::target(AllocationId::Stack(place.local), Interval::new(0, 0)),
        )
    };

    let Some(delta) = projection_offset(
        tcx,
        local_decls,
        place,
        base,
        start_projection,
        integer_value,
    ) else {
        return value.forget_offsets();
    };
    value = value.add_offset(delta);
    value
}

fn projection_offset<'tcx>(
    tcx: TyCtxt<'tcx>,
    local_decls: &LocalDecls<'tcx>,
    full_place: Place<'tcx>,
    mut base: Place<'tcx>,
    start_projection: usize,
    integer_value: &mut impl FnMut(&Operand<'tcx>) -> Interval,
) -> Option<Interval> {
    let mut offset = Interval::new(0, 0);
    for elem in full_place.projection.iter().skip(start_projection) {
        let base_ty = base.ty(local_decls, tcx).ty;
        let delta = match elem {
            ProjectionElem::Field(field, _) => {
                let layout = tcx
                    .layout_of(TypingEnv::fully_monomorphized().as_query_input(base_ty))
                    .ok()?;
                let bytes = layout.fields.offset(field.index()).bytes() as i128;
                Interval::new(bytes, bytes)
            }
            ProjectionElem::Index(local) => {
                let element_size = sequence_element_size(tcx, base_ty)?;
                mul(
                    &integer_value(&Operand::Copy(Place::from(local))),
                    &element_size,
                )
            }
            ProjectionElem::ConstantIndex {
                offset: index,
                from_end,
                ..
            } => {
                let index = if from_end {
                    let TyKind::Array(_, len) = base_ty.kind() else {
                        return None;
                    };
                    len.try_to_target_usize(tcx)?.checked_sub(index)?
                } else {
                    index
                };
                mul(
                    &Interval::new(index as i128, index as i128),
                    &sequence_element_size(tcx, base_ty)?,
                )
            }
            ProjectionElem::Subslice { from, .. } => mul(
                &Interval::new(from as i128, from as i128),
                &sequence_element_size(tcx, base_ty)?,
            ),
            ProjectionElem::Deref => return None,
            ProjectionElem::Downcast(_, _)
            | ProjectionElem::OpaqueCast(_)
            | ProjectionElem::Subtype(_)
            | ProjectionElem::UnwrapUnsafeBinder(_) => Interval::new(0, 0),
        };
        offset = crate::interval::abstract_value::add(&offset, &delta);
        base = base.project_deeper(&[elem], tcx);
    }
    Some(offset)
}

fn sequence_element_size<'tcx>(tcx: TyCtxt<'tcx>, ty: Ty<'tcx>) -> Option<Interval> {
    match ty.kind() {
        TyKind::Array(element, _) | TyKind::Slice(element) => type_size(tcx, *element),
        _ => None,
    }
}

fn pointer_element_size<'tcx>(tcx: TyCtxt<'tcx>, ty: Ty<'tcx>) -> Interval {
    pointer_pointee_type(tcx, ty)
        .and_then(|pointee| type_size(tcx, pointee))
        .unwrap_or_else(Interval::top)
}

fn heap_id<'tcx>(
    term: &Terminator<'tcx>,
    symbolic: &SymbolicState<'tcx>,
    state: &AllocationState<'tcx>,
    destination: Place<'tcx>,
) -> AllocationId {
    let destination = state
        .pointer_path_resolved(symbolic, destination)
        .or_else(|| AccessPath::from_place(destination))
        .unwrap_or_else(|| AccessPath::from_local(destination.local));
    AllocationId::Heap(AllocationSite {
        span: term.source_info.span,
        destination,
    })
}

fn first_deallocation_pointer_argument<'a, 'tcx>(
    tcx: TyCtxt<'tcx>,
    local_decls: &LocalDecls<'tcx>,
    args: &'a [Spanned<Operand<'tcx>>],
) -> Option<&'a Operand<'tcx>> {
    args.iter()
        .find(|arg| {
            let ty = arg.node.ty(local_decls, tcx);
            matches!(ty.kind(), TyKind::RawPtr(_, _)) || is_non_null_ty(tcx, ty)
        })
        .map(|arg| &arg.node)
}

fn call_def<'tcx>(
    tcx: TyCtxt<'tcx>,
    local_decls: &LocalDecls<'tcx>,
    func: &Operand<'tcx>,
) -> Option<(
    rustc_hir::def_id::DefId,
    rustc_middle::ty::GenericArgsRef<'tcx>,
)> {
    let TyKind::FnDef(def_id, args) = func.ty(local_decls, tcx).kind() else {
        return None;
    };
    Some((*def_id, args))
}

fn is_integer(ty: Ty<'_>) -> bool {
    matches!(ty.kind(), TyKind::Int(_) | TyKind::Uint(_))
}

fn is_allocator_api(path: &str) -> bool {
    path.ends_with("::alloc::alloc")
        || path.ends_with("::alloc::alloc_zeroed")
        || path.ends_with("::alloc::dealloc")
        || path.ends_with("::alloc::realloc")
        || path.contains("::alloc::alloc::")
        || path.contains("::alloc::GlobalAlloc::")
        || path.contains("::alloc::Allocator::")
        || path.contains("::alloc::System::")
        || path.contains("::GlobalAlloc::")
}

fn is_allocate_call(path: &str) -> bool {
    is_allocator_api(path) && (path.ends_with("::alloc") || path.ends_with("::alloc_zeroed"))
}

fn is_deallocate_call(path: &str) -> bool {
    is_allocator_api(path) && (path.ends_with("::dealloc") || path.ends_with("::deallocate"))
}

fn is_reallocate_call(path: &str) -> bool {
    is_allocator_api(path) && path.ends_with("::realloc")
}

fn is_pointer_offset_call(path: &str) -> bool {
    [
        "::add",
        "::sub",
        "::offset",
        "::wrapping_add",
        "::wrapping_sub",
        "::wrapping_offset",
        "::byte_add",
        "::byte_sub",
        "::byte_offset",
        "::wrapping_byte_add",
        "::wrapping_byte_sub",
        "::wrapping_byte_offset",
    ]
    .iter()
    .any(|suffix| path.ends_with(suffix))
}

fn is_byte_offset_call(path: &str) -> bool {
    path.ends_with("::byte_add")
        || path.ends_with("::byte_sub")
        || path.ends_with("::byte_offset")
        || path.ends_with("::wrapping_byte_add")
        || path.ends_with("::wrapping_byte_sub")
        || path.ends_with("::wrapping_byte_offset")
}

fn is_subtracting_offset_call(path: &str) -> bool {
    path.ends_with("::sub")
        || path.ends_with("::wrapping_sub")
        || path.ends_with("::byte_sub")
        || path.ends_with("::wrapping_byte_sub")
}

fn is_pointer_difference_call(path: &str) -> bool {
    path.ends_with("::offset_from")
        || path.ends_with("::offset_from_unsigned")
        || path.ends_with("::byte_offset_from")
        || path.ends_with("::byte_offset_from_unsigned")
        || path.ends_with("::sub_ptr")
}

fn is_byte_difference_call(path: &str) -> bool {
    path.ends_with("::byte_offset_from") || path.ends_with("::byte_offset_from_unsigned")
}
