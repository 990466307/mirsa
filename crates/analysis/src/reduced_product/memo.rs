//! Cache only reductions that have reached a local fixed point.
use super::CombinedState;
use mirsa_framework::reduction::MAX_VALUES;
use mirsa_framework::revision::Revision;
use rustc_middle::mir::{LocalDecls, Place};
use std::collections::HashSet;

type Inputs = (
    [Revision; 13],
    [Revision; 7],
    [Revision; 2],
    ([Revision; 7], bool),
);

#[derive(Debug, Default)]
pub(super) struct ReductionMemo<'tcx> {
    context: (usize, usize),
    inputs: Option<Inputs>,
    locals: HashSet<Place<'tcx>>,
    roots: HashSet<Place<'tcx>>,
}

// Memoization belongs to the live state, not snapshots, joins or summaries.
impl Clone for ReductionMemo<'_> {
    fn clone(&self) -> Self {
        Self::default()
    }
}

impl<'tcx> ReductionMemo<'tcx> {
    // States and their access paths belong to one MIR body. Projection creates
    // a fresh memo; the declaration identity also guards replay contexts.
    pub fn prepare(&mut self, decls: &LocalDecls<'tcx>, inputs: Inputs) {
        let context = (std::ptr::from_ref(decls) as *const () as usize, decls.len());
        if self.context != context || self.inputs != Some(inputs) {
            self.locals.clear();
            self.roots.clear();
            self.context = context;
            self.inputs = Some(inputs);
        }
    }
    pub fn local(&self, place: Place<'tcx>) -> bool {
        self.locals.contains(&place)
    }
    pub fn root(&self, place: Place<'tcx>) -> bool {
        self.roots.contains(&place)
    }
    pub fn remember_local(&mut self, place: Place<'tcx>) {
        if self.locals.len() < MAX_VALUES {
            self.locals.insert(place);
        }
    }
    pub fn remember_root(&mut self, place: Place<'tcx>) {
        if self.roots.len() < MAX_VALUES {
            self.roots.insert(place);
        }
    }
}

impl CombinedState<'_> {
    pub(super) fn reduction_revision(&self) -> Inputs {
        (
            self.symbolic.revision(),
            self.interval.revision(),
            self.nullptr.revision(),
            self.allocation.revision(),
        )
    }
}
