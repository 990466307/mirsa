mod copy_nonoverlapping;
mod nonzero;
mod shared;
mod slice_bounds;

use rustc_middle::mir::Body;
use rustc_middle::ty::TyCtxt;

use crate::contracts::emit_call_findings;
use crate::contracts::matcher::{ContractCall, classify_call};
use crate::framework::forward::PathForwardAnalysisResult;
use crate::internval::InternvalState;
use crate::internval::engine::state_before_location;

pub fn is_supported_unsafe_call<'tcx>(
    tcx: TyCtxt<'tcx>,
    body: &Body<'tcx>,
    term: &rustc_middle::mir::Terminator<'tcx>,
) -> bool {
    classify_call(tcx, body, term).is_some_and(ContractCall::has_internval_contract)
}

pub fn emit_internval_warnings<'tcx>(
    tcx: TyCtxt<'tcx>,
    body: &Body<'tcx>,
    result: &PathForwardAnalysisResult<InternvalState<'tcx>>,
) {
    emit_call_findings(
        tcx,
        body,
        result,
        state_before_location,
        |tcx, body, term, state, call| match call {
            ContractCall::NonZeroNewUnchecked => nonzero::check(tcx, body, term, state),
            ContractCall::SliceGetUnchecked
            | ContractCall::SliceGetUncheckedMut
            | ContractCall::SliceSplitAtUnchecked
            | ContractCall::SliceSplitAtMutUnchecked => {
                slice_bounds::check(tcx, body, term, state, call)
            }
            ContractCall::PtrCopyNonoverlapping => {
                copy_nonoverlapping::check(tcx, body, term, state)
            }
            _ => None,
        },
    );
}
