mod copy_nonoverlapping;
mod nonnull_arg;
mod shared;

use rustc_middle::mir::Body;
use rustc_middle::ty::TyCtxt;

use crate::contracts::emit_call_findings;
use crate::contracts::matcher::{ContractCall, classify_call};
use crate::framework::forward::PathForwardAnalysisResult;
use crate::nullptr::NullPtrState;
use crate::nullptr::engine::state_before_location;

pub fn is_supported_unsafe_call<'tcx>(
    tcx: TyCtxt<'tcx>,
    body: &Body<'tcx>,
    term: &rustc_middle::mir::Terminator<'tcx>,
) -> bool {
    classify_call(tcx, body, term).is_some_and(ContractCall::has_nullptr_contract)
}

pub fn emit_nonnull_call_warnings<'tcx>(
    tcx: TyCtxt<'tcx>,
    body: &Body<'tcx>,
    result: &PathForwardAnalysisResult<NullPtrState<'tcx>>,
    warn_on_maybe: bool,
) {
    emit_call_findings(
        tcx,
        body,
        result,
        state_before_location,
        |tcx, body, term, state, call| match call {
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
        },
    );
}
