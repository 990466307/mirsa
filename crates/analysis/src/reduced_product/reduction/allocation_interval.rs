use mirsa_domains::allocation::AllocationState;
use mirsa_domains::allocation::transfer::{
    layout_scalar_call_result, pointer_difference_call_result,
};
use mirsa_domains::interval::IntervalState;
use mirsa_relations::symbolic::SymbolicState;
use rustc_middle::mir::{LocalDecls, Terminator, TerminatorKind};
use rustc_middle::ty::TyCtxt;

pub fn reduce_call_result<'tcx>(
    tcx: TyCtxt<'tcx>,
    local_decls: &LocalDecls<'tcx>,
    interval: &mut IntervalState<'tcx>,
    allocation: &AllocationState<'tcx>,
    symbolic: &SymbolicState<'tcx>,
    term: &Terminator<'tcx>,
) {
    let TerminatorKind::Call { destination, .. } = &term.kind else {
        return;
    };
    let value = layout_scalar_call_result(tcx, symbolic, allocation, term, local_decls)
        .or_else(|| pointer_difference_call_result(tcx, symbolic, allocation, term, local_decls));
    let Some(value) = value else {
        return;
    };
    interval.debug(format_args!(
        "reduce scalar from allocation {:?}: {}",
        destination, value
    ));
    interval.set_interval_resolved(symbolic, *destination, value);
}
