mod copy_nonoverlapping;
mod nonzero;
mod shared;
mod slice_bounds;

use crate::finding::Finding;
use crate::matcher::{ContractCall, classify_call};
use mirsa_domains::interval::IntervalState;
use rustc_middle::mir::Body;
use rustc_middle::ty::TyCtxt;

pub fn is_supported_unsafe_call<'tcx>(
    tcx: TyCtxt<'tcx>,
    body: &Body<'tcx>,
    term: &rustc_middle::mir::Terminator<'tcx>,
) -> bool {
    classify_call(tcx, body, term).is_some_and(ContractCall::has_interval_contract)
}

pub fn check_interval_call<'tcx>(
    tcx: TyCtxt<'tcx>,
    body: &Body<'tcx>,
    term: &rustc_middle::mir::Terminator<'tcx>,
    state: &IntervalState<'tcx>,
    call: ContractCall,
) -> Option<Finding> {
    match call {
        ContractCall::NonZeroNewUnchecked => nonzero::check(tcx, body, term, state),
        ContractCall::SliceGetUnchecked
        | ContractCall::SliceGetUncheckedMut
        | ContractCall::SliceSplitAtUnchecked
        | ContractCall::SliceSplitAtMutUnchecked => {
            slice_bounds::check(tcx, body, term, state, call)
        }
        ContractCall::PtrCopyNonoverlapping => copy_nonoverlapping::check(tcx, body, term, state),
        _ => None,
    }
}
