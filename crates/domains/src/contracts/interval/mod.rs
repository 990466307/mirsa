mod copy_nonoverlapping;
mod nonzero;
mod shared;
mod slice_bounds;

use rustc_middle::mir::Body;
use rustc_middle::ty::TyCtxt;

use crate::contracts::emit_call_findings;
use crate::contracts::finding::Finding;
use crate::contracts::matcher::{ContractCall, classify_call};
use crate::framework::forward::PathForwardAnalysisResult;
use crate::interval::engine::state_before_location;
use crate::interval::{IntervalAnalysisState, IntervalState};

pub fn is_supported_unsafe_call<'tcx>(
    tcx: TyCtxt<'tcx>,
    body: &Body<'tcx>,
    term: &rustc_middle::mir::Terminator<'tcx>,
) -> bool {
    classify_call(tcx, body, term).is_some_and(ContractCall::has_interval_contract)
}

pub fn emit_interval_warnings<'tcx>(
    tcx: TyCtxt<'tcx>,
    body: &Body<'tcx>,
    result: &PathForwardAnalysisResult<IntervalAnalysisState<'tcx>>,
) {
    emit_call_findings(
        tcx,
        body,
        result,
        state_before_location,
        |tcx, body, term, state, call| check_interval_call(tcx, body, term, &state.interval, call),
    );
}

pub(crate) fn check_interval_call<'tcx>(
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
