mod nonzero;
mod shared;
mod slice_bounds;

use rustc_middle::mir::{Body, TerminatorKind};
use rustc_middle::ty::{TyCtxt, TyKind};

use crate::internval::InternvalState;

fn call_path<'tcx>(
    tcx: TyCtxt<'tcx>,
    body: &'tcx Body<'tcx>,
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
    body: &'tcx Body<'tcx>,
    term: &rustc_middle::mir::Terminator<'tcx>,
) -> bool {
    let Some(path) = call_path(tcx, body, term) else {
        return false;
    };
    nonzero::matches_path(&path) || slice_bounds::matches_path(&path)
}

pub fn emit_internval_warnings<'tcx>(
    tcx: TyCtxt<'tcx>,
    body: &'tcx Body<'tcx>,
    states: &[InternvalState<'tcx>],
) {
    for (bb, bbdata) in body.basic_blocks.iter_enumerated() {
        let Some(term) = bbdata.terminator.as_ref() else {
            continue;
        };
        let Some(path) = call_path(tcx, body, term) else {
            continue;
        };
        let Some(state) = states.get(bb.index()) else {
            continue;
        };

        if nonzero::matches_path(&path) {
            nonzero::emit(tcx, body, term, state);
        } else if slice_bounds::matches_path(&path) {
            slice_bounds::emit(tcx, body, term, state, &path);
        }
    }
}
