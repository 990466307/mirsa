use rustc_middle::mir::{Body, Location};
use rustc_middle::ty::TyCtxt;

use crate::reduced_product::CombinedState;
use crate::reduced_product::engine::state_before_location;
use mirsa_contracts::finding::emit_finding;
use mirsa_contracts::interval::check_interval_call;
use mirsa_contracts::matcher::classify_call;
use mirsa_contracts::nullptr::check_nullptr_call;
use mirsa_framework::forward::PathForwardAnalysisResult;

pub fn emit_combined_warnings<'tcx>(
    tcx: TyCtxt<'tcx>,
    body: &Body<'tcx>,
    result: &PathForwardAnalysisResult<CombinedState<'tcx>>,
    warn_on_maybe: bool,
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
        let Some(state) = state_before_location(tcx, body, result, location) else {
            continue;
        };
        if let Some(finding) = check_interval_call(tcx, body, term, &state.interval, call) {
            emit_finding(tcx, &finding);
        }
        if let Some(finding) =
            check_nullptr_call(tcx, body, term, &state.nullptr, call, warn_on_maybe)
        {
            emit_finding(tcx, &finding);
        }
    }
}
