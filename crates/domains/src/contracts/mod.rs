pub mod combined;
pub mod finding;
pub mod interval;
pub mod matcher;
pub mod nullptr;

use rustc_middle::mir::{Body, Location, Terminator};
use rustc_middle::ty::TyCtxt;

use crate::contracts::finding::{Finding, emit_finding};
use crate::contracts::matcher::{ContractCall, classify_call};
use crate::framework::forward::PathForwardAnalysisResult;

pub(crate) fn emit_call_findings<'tcx, S>(
    tcx: TyCtxt<'tcx>,
    body: &Body<'tcx>,
    result: &PathForwardAnalysisResult<S>,
    mut state_before: impl FnMut(
        TyCtxt<'tcx>,
        &Body<'tcx>,
        &PathForwardAnalysisResult<S>,
        Location,
    ) -> Option<S>,
    mut check: impl FnMut(
        TyCtxt<'tcx>,
        &Body<'tcx>,
        &Terminator<'tcx>,
        &S,
        ContractCall,
    ) -> Option<Finding>,
) {
    for (bb, bbdata) in body.basic_blocks.iter_enumerated() {
        let Some(term) = bbdata.terminator.as_ref() else {
            continue;
        };
        let Some(call) = classify_call(tcx, body, term) else {
            continue;
        };
        let location = Location {
            block: bb,
            statement_index: bbdata.statements.len(),
        };
        let Some(state) = state_before(tcx, body, result, location) else {
            continue;
        };
        if let Some(finding) = check(tcx, body, term, &state, call) {
            emit_finding(tcx, &finding);
        };
    }
}
