use mirsa_framework::reduction::MAX_RELATED;
use mirsa_relations::symbolic::{SymbolicExpr, SymbolicState};
use rustc_middle::mir::Place;

pub(super) mod allocation_interval;
pub(super) mod allocation_nullptr;
pub(super) mod equivalent_values;
mod pointer_relations;
mod refine;
pub(super) mod symbolic_allocation;
pub(super) mod symbolic_interval;
pub(super) mod symbolic_nullptr;
pub(super) mod value_relations;

fn expression<'a, 'tcx>(
    symbolic: &'a SymbolicState<'tcx>,
    place: Place<'tcx>,
) -> Option<&'a SymbolicExpr<'tcx>> {
    symbolic
        .expr_for_place(place)
        .filter(|expr| !matches!(expr, SymbolicExpr::Call { args, .. } if args.len() > MAX_RELATED))
}
