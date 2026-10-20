use super::abstract_value::PointerValue;
use super::state::{AllocationState, is_vec_ty, type_size};
use super::transfer::{eval_pointer_operand, heap_id};
use crate::interval::abstract_value::{Interval, mul};
use mirsa_framework::access_path::AccessPath;
use mirsa_framework::call_model::BufferCall;
use mirsa_relations::symbolic::SymbolicState;
use rustc_middle::mir::*;
use rustc_middle::ty::{Ty, TyCtxt, TyKind};
use rustc_span::source_map::Spanned;

pub(super) fn assign<'tcx>(
    tcx: TyCtxt<'tcx>,
    symbolic: &SymbolicState<'tcx>,
    state: &mut AllocationState<'tcx>,
    destination: Place<'tcx>,
    rvalue: &Rvalue<'tcx>,
    local_decls: &LocalDecls<'tcx>,
) -> bool {
    if !destination.ty(local_decls, tcx).ty.is_box() {
        return false;
    }
    let value = match rvalue {
        Rvalue::ShallowInitBox(operand, _) => eval_pointer_operand(
            tcx,
            symbolic,
            state,
            operand,
            local_decls,
            operand.ty(local_decls, tcx),
        ),
        Rvalue::Use(operand) | Rvalue::Cast(_, operand, _) => {
            let value = operand_buffer(symbolic, state, operand);
            state.assign_from_operand(symbolic, destination, operand);
            value
        }
        Rvalue::CopyForDeref(source) => {
            let operand = Operand::Copy(*source);
            let value = operand_buffer(symbolic, state, &operand);
            state.assign_from_operand(symbolic, destination, &operand);
            value
        }
        _ => PointerValue::unknown_non_null(),
    };
    if let Some(path) = state.pointer_path_resolved(symbolic, destination) {
        state.set_buffer(path, value);
    }
    true
}

pub(super) fn apply<'tcx>(
    tcx: TyCtxt<'tcx>,
    symbolic: &SymbolicState<'tcx>,
    state: &mut AllocationState<'tcx>,
    term: &Terminator<'tcx>,
    local_decls: &LocalDecls<'tcx>,
    integer_value: &mut impl FnMut(&Operand<'tcx>) -> Interval,
) -> bool {
    if let TerminatorKind::Drop { place, .. } = &term.kind {
        drop_place(tcx, symbolic, state, *place, local_decls);
        return true;
    }
    let TerminatorKind::Call {
        func,
        args,
        destination,
        ..
    } = &term.kind
    else {
        return false;
    };
    let model = BufferCall::classify(tcx, func, local_decls);
    let destination_ty = destination.ty(local_decls, tcx).ty;
    match model {
        BufferCall::Drop => {
            if let Some(place) = args.first().and_then(|arg| arg.node.place()) {
                drop_place(tcx, symbolic, state, place, local_decls);
            }
            return true;
        }
        BufferCall::AllocateBox => {
            let extent = args
                .first()
                .map(|arg| integer_value(&arg.node))
                .unwrap_or_else(Interval::top);
            let value = state.allocate_buffer(heap_id(term, symbolic, state, *destination), extent);
            state.set_pointer_resolved(symbolic, *destination, value);
            return true;
        }
        BufferCall::VecData => {
            let value = args
                .first()
                .and_then(|arg| arg.node.place())
                .and_then(AccessPath::from_place)
                .map(|path| state.buffer(&symbolic.normalize_path(&path.deref())))
                .unwrap_or_else(PointerValue::unknown_non_null);
            state.set_pointer_resolved(symbolic, *destination, value);
            return true;
        }
        _ => {}
    }
    if is_vec_ty(tcx, destination_ty) {
        let value = match model {
            BufferCall::BoxIntoVec => args
                .first()
                .map(|arg| operand_buffer(symbolic, state, &arg.node))
                .unwrap_or_else(PointerValue::unknown_non_null),
            BufferCall::EmptyVec => PointerValue::invalid_non_null(),
            BufferCall::AllocateVec { count_argument } => {
                let count = args
                    .get(count_argument)
                    .map(|arg| integer_value(&arg.node))
                    .unwrap_or_else(Interval::top);
                let TyKind::Adt(_, types) = destination_ty.kind() else {
                    unreachable!()
                };
                let size = types
                    .types()
                    .next()
                    .and_then(|ty| type_size(tcx, ty))
                    .unwrap_or_else(Interval::top);
                let extent = if size.high == 0 {
                    Interval::new(0, 0)
                } else {
                    Interval::new(mul(&count, &size).low.max(0), i128::MAX)
                };
                state.allocate_buffer(heap_id(term, symbolic, state, *destination), extent)
            }
            BufferCall::VecFromRawParts => args
                .first()
                .map(|arg| {
                    eval_pointer_operand(
                        tcx,
                        symbolic,
                        state,
                        &arg.node,
                        local_decls,
                        arg.node.ty(local_decls, tcx),
                    )
                })
                .unwrap_or_else(PointerValue::unknown_non_null),
            _ => {
                consume_arguments(symbolic, state, args);
                PointerValue::unknown_non_null()
            }
        };
        if let Some(path) = state.pointer_path_resolved(symbolic, *destination) {
            state.set_buffer(path, value);
        }
        if model == BufferCall::BoxIntoVec {
            if let Some(Spanned {
                node: Operand::Move(source),
                ..
            }) = args.first()
            {
                if let Some(path) = state.pointer_path_resolved(symbolic, *source) {
                    state.forget_buffers(&path);
                }
            }
        }
        return true;
    }
    if !matches!(model, BufferCall::Forget | BufferCall::VecIntoRawParts) {
        consume_arguments(symbolic, state, args);
    }
    if destination_ty.is_box() && !matches!(func.ty(local_decls, tcx).kind(), TyKind::FnDef(..)) {
        if let Some(path) = state.pointer_path_resolved(symbolic, *destination) {
            state.set_buffer(path, PointerValue::unknown_non_null());
        }
    }
    false
}

fn operand_buffer<'tcx>(
    symbolic: &SymbolicState<'tcx>,
    state: &AllocationState<'tcx>,
    operand: &Operand<'tcx>,
) -> PointerValue {
    operand
        .place()
        .and_then(|place| state.pointer_path_resolved(symbolic, place))
        .map(|path| state.buffer(&path))
        .unwrap_or_else(PointerValue::unknown_non_null)
}

fn consume_arguments<'tcx>(
    symbolic: &SymbolicState<'tcx>,
    state: &mut AllocationState<'tcx>,
    args: &[Spanned<Operand<'tcx>>],
) {
    for arg in args {
        if let Operand::Move(place) = &arg.node {
            if let Some(path) = state.pointer_path_resolved(symbolic, *place) {
                state.drop_buffers(&path, true);
            }
        }
    }
}

fn drop_place<'tcx>(
    tcx: TyCtxt<'tcx>,
    symbolic: &SymbolicState<'tcx>,
    state: &mut AllocationState<'tcx>,
    place: Place<'tcx>,
    local_decls: &LocalDecls<'tcx>,
) {
    if let Some(path) = state.pointer_path_resolved(symbolic, place) {
        state.drop_buffers(
            &path,
            drop_may_transfer_buffer(tcx, place.ty(local_decls, tcx).ty),
        );
    }
}

fn drop_may_transfer_buffer<'tcx>(tcx: TyCtxt<'tcx>, ty: Ty<'tcx>) -> bool {
    match ty.kind() {
        TyKind::Adt(_, _) if is_vec_ty(tcx, ty) || ty.is_box() => false,
        TyKind::Adt(def, args) => {
            def.has_dtor(tcx)
                || def
                    .all_fields()
                    .any(|field| drop_may_transfer_buffer(tcx, field.ty(tcx, args)))
        }
        TyKind::Tuple(types) => types.iter().any(|ty| drop_may_transfer_buffer(tcx, ty)),
        TyKind::Array(element, _) => drop_may_transfer_buffer(tcx, *element),
        _ => false,
    }
}
