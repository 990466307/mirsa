use super::{Interval, state::IntervalState};
use mirsa_relations::symbolic::SymbolicState;
use rustc_abi::Size;
use rustc_middle::mir::interpret::{GlobalAlloc, Scalar, alloc_range};
use rustc_middle::mir::{ConstOperand, ConstValue, Place, ProjectionElem};
use rustc_middle::ty::{Mutability, TyCtxt, TyKind, TypingEnv, UintTy};

/// Materialize small immutable byte constants just like explicit byte-array aggregates.
/// Bound the work independently of the size of a string embedded in the program.
pub(super) fn assign<'tcx>(
    tcx: TyCtxt<'tcx>,
    symbolic: &SymbolicState<'tcx>,
    state: &mut IntervalState<'tcx>,
    destination: Place<'tcx>,
    constant: &ConstOperand<'tcx>,
) {
    let Some(bytes) = read(tcx, constant) else {
        return;
    };
    let len = bytes.len() as u64;
    state.set_len_resolved(
        symbolic,
        destination,
        Interval::new(len as i128, len as i128),
    );
    for (index, &byte) in bytes.iter().enumerate() {
        let element = destination.project_deeper(
            &[
                ProjectionElem::Deref,
                ProjectionElem::ConstantIndex {
                    offset: index as u64,
                    min_length: len,
                    from_end: false,
                },
            ],
            tcx,
        );
        state.set_interval_resolved(symbolic, element, Interval::new(byte as i128, byte as i128));
    }
}

fn read<'tcx>(tcx: TyCtxt<'tcx>, constant: &ConstOperand<'tcx>) -> Option<&'tcx [u8]> {
    const MAX_BYTES: u64 = 256;
    let TyKind::Ref(_, inner, Mutability::Not) = constant.ty().kind() else {
        return None;
    };
    let byte = |ty: rustc_middle::ty::Ty<'tcx>| matches!(ty.kind(), TyKind::Uint(UintTy::U8));
    let fixed_len = match inner.kind() {
        TyKind::Array(element, len) if byte(*element) => Some(len.try_to_target_usize(tcx)?),
        TyKind::Slice(element) if byte(*element) => None,
        _ => return None,
    };
    let value = constant
        .const_
        .eval(tcx, TypingEnv::fully_monomorphized(), constant.span)
        .ok()?;
    let (allocation, offset, len) = match value {
        ConstValue::Slice { data, meta } => (data, Size::ZERO, meta),
        ConstValue::Scalar(Scalar::Ptr(pointer, _)) => {
            let (provenance, offset) = pointer.into_parts();
            let GlobalAlloc::Memory(allocation) = tcx.global_alloc(provenance.alloc_id()) else {
                return None;
            };
            (allocation, offset, fixed_len?)
        }
        _ => return None,
    };
    let allocation = allocation.inner();
    if len > MAX_BYTES
        || allocation.mutability != Mutability::Not
        || offset.bytes().checked_add(len)? > allocation.size().bytes()
    {
        return None;
    }
    allocation
        .get_bytes_strip_provenance(&tcx, alloc_range(offset, Size::from_bytes(len)))
        .ok()
}
