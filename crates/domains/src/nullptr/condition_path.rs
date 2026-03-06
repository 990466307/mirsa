use rustc_middle::mir::{BasicBlock, Body};
use rustc_middle::ty::TyCtxt;

use super::state::NullPtrState;

pub fn refine_edge<'tcx>(
    _tcx: TyCtxt<'tcx>,
    _body: &'tcx Body<'tcx>,
    _pred: BasicBlock,
    _succ: BasicBlock,
    in_state: &NullPtrState<'tcx>,
) -> Option<NullPtrState<'tcx>> {
    Some(in_state.clone())
}
