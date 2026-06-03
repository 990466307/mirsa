mod copy_nonoverlapping;
mod nonnull_arg;
mod shared;

use crate::finding::Finding;
use crate::matcher::{ContractCall, classify_call};
use mirsa_domains::nullptr::NullPtrState;
use rustc_middle::mir::Body;
use rustc_middle::ty::TyCtxt;

pub fn is_supported_unsafe_call<'tcx>(
    tcx: TyCtxt<'tcx>,
    body: &Body<'tcx>,
    term: &rustc_middle::mir::Terminator<'tcx>,
) -> bool {
    classify_call(tcx, body, term).is_some_and(ContractCall::has_nullptr_contract)
}

pub fn check_nullptr_call<'tcx>(
    tcx: TyCtxt<'tcx>,
    body: &Body<'tcx>,
    term: &rustc_middle::mir::Terminator<'tcx>,
    state: &NullPtrState<'tcx>,
    call: ContractCall,
    warn_on_maybe: bool,
) -> Option<Finding> {
    match call {
        ContractCall::PtrCopyNonoverlapping => {
            copy_nonoverlapping::check(tcx, body, term, state, warn_on_maybe)
        }
        ContractCall::NonNullNewUnchecked
        | ContractCall::CStrFromPtr
        | ContractCall::SliceFromRawParts
        | ContractCall::SliceFromRawPartsMut
        | ContractCall::VecFromRawParts
        | ContractCall::PtrRead
        | ContractCall::PtrWrite => nonnull_arg::check(tcx, body, term, state, warn_on_maybe),
        _ => None,
    }
}
