mod copy_nonoverlapping;
mod nonzero;
mod shared;
mod slice_bounds;

use rustc_middle::mir::Body;
use rustc_middle::ty::TyCtxt;

use crate::framework::forward::PathForwardAnalysisResult;
use crate::internval::InternvalState;
use crate::internval::engine::state_before_location;

use self::shared::call_path;

pub fn is_supported_unsafe_call<'tcx>(
    tcx: TyCtxt<'tcx>,
    body: &Body<'tcx>,
    term: &rustc_middle::mir::Terminator<'tcx>,
) -> bool {
    let Some(path) = call_path(tcx, body, term) else {
        return false;
    };
    nonzero::matches_path(&path)
        || slice_bounds::matches_path(&path)
        || copy_nonoverlapping::matches_path(&path)
}

pub fn emit_internval_warnings<'tcx>(
    tcx: TyCtxt<'tcx>,
    body: &Body<'tcx>,
    result: &PathForwardAnalysisResult<InternvalState<'tcx>>,
) {
    for (bb, bbdata) in body.basic_blocks.iter_enumerated() {
        let Some(term) = bbdata.terminator.as_ref() else {
            continue;
        };
        let Some(path) = call_path(tcx, body, term) else {
            continue;
        };
        let location = rustc_middle::mir::Location {
            block: bb,
            statement_index: bbdata.statements.len(),
        };
        let Some(state) = state_before_location(tcx, body, result, location) else {
            continue;
        };

        if nonzero::matches_path(&path) {
            nonzero::emit(tcx, body, term, &state);
        } else if slice_bounds::matches_path(&path) {
            slice_bounds::emit(tcx, body, term, &state, &path);
        } else if copy_nonoverlapping::matches_path(&path) {
            copy_nonoverlapping::emit(tcx, body, term, &state);
        }
    }
}
