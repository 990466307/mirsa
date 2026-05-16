mod copy_nonoverlapping;
mod nonnull_arg;
mod shared;

use rustc_middle::mir::{Body, TerminatorKind};
use rustc_middle::ty::{TyCtxt, TyKind};

use crate::framework::forward::PathForwardAnalysisResult;
use crate::nullptr::NullPtrState;
use crate::nullptr::engine::state_before_location;

fn call_path<'tcx>(
    tcx: TyCtxt<'tcx>,
    body: &Body<'tcx>,
    term: &rustc_middle::mir::Terminator<'tcx>,
) -> Option<String> {
    let TerminatorKind::Call { func, .. } = &term.kind else {
        return None;
    };
    let TyKind::FnDef(def_id, _) = func.ty(&body.local_decls, tcx).kind() else {
        return None;
    };
    Some(tcx.def_path_str(*def_id))
}

pub fn is_supported_unsafe_call<'tcx>(
    tcx: TyCtxt<'tcx>,
    body: &Body<'tcx>,
    term: &rustc_middle::mir::Terminator<'tcx>,
) -> bool {
    let Some(path) = call_path(tcx, body, term) else {
        return false;
    };
    nonnull_arg::matches_path(&path) || copy_nonoverlapping::matches_path(&path)
}

pub fn emit_nonnull_call_warnings<'tcx>(
    tcx: TyCtxt<'tcx>,
    body: &Body<'tcx>,
    result: &PathForwardAnalysisResult<NullPtrState<'tcx>>,
    warn_on_maybe: bool,
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

        if copy_nonoverlapping::matches_path(&path) {
            copy_nonoverlapping::emit(tcx, body, term, &state, warn_on_maybe);
        } else if nonnull_arg::matches_path(&path) {
            nonnull_arg::emit(tcx, body, term, &state, warn_on_maybe);
        }
    }
}
