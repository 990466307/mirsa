use super::state::CombinedState;
use mirsa_framework::access_path::AccessPath;
use mirsa_framework::reduction::{MAX_DEPTH, MAX_PASSES, MAX_RELATED, MAX_VALUES};
use rustc_middle::mir::visit::{PlaceContext, Visitor};
use rustc_middle::mir::{LocalDecls, Location, Place, ProjectionElem, Statement, Terminator};
use rustc_middle::ty::TyCtxt;
use std::collections::HashSet;

impl<'tcx> CombinedState<'tcx> {
    pub fn reduce_place(
        &mut self,
        tcx: TyCtxt<'tcx>,
        local_decls: &LocalDecls<'tcx>,
        place: Place<'tcx>,
    ) -> bool {
        if !self.feasible {
            return false;
        }
        let initial = self.reduction_revision();
        self.reduction_memo.prepare(local_decls, initial);
        if self.reduction_memo.root(place) {
            return true;
        }
        let mut pending = vec![(place, 0)];
        let mut seen = HashSet::<AccessPath>::new();
        if let Some(path) = self.symbolic.normalize_place(place) {
            seen.insert(path);
        }
        let mut cursor = 0;
        while cursor < pending.len() {
            let (current, depth) = pending[cursor];
            cursor += 1;
            if depth == MAX_DEPTH || pending.len() == MAX_VALUES {
                continue;
            }
            for related in self.symbolic.related_places(current, MAX_RELATED) {
                let Some(path) = self.symbolic.normalize_place(related) else {
                    continue;
                };
                if seen.insert(path) {
                    pending.push((related, depth + 1));
                }
                if pending.len() == MAX_VALUES {
                    break;
                }
            }
        }
        self.debug_reduce(format_args!("demand {place:?}: {} values", pending.len()));
        // A bounded pass is not necessarily a fixed point. Memoize only local
        // evaluations that did not change any input fact. Any domain mutation
        // invalidates them, including changes made by another local reduction.
        let mut revision = initial;
        for _ in 0..MAX_PASSES {
            for &(place, _) in pending.iter().rev().chain(pending.iter()) {
                if self.reduction_memo.local(place) {
                    continue;
                }
                if !self.reduce_local(tcx, local_decls, place) {
                    self.feasible = false;
                    return false;
                }
                let after = self.reduction_revision();
                if after == revision {
                    self.reduction_memo.remember_local(place);
                } else {
                    self.reduction_memo.prepare(local_decls, after);
                    revision = after;
                }
            }
        }
        if initial == revision {
            self.reduction_memo.remember_root(place);
        }
        true
    }

    pub(super) fn demand_statement(
        &mut self,
        tcx: TyCtxt<'tcx>,
        decls: &LocalDecls<'tcx>,
        statement: &Statement<'tcx>,
    ) -> bool {
        let mut reads = ReadPlaces::new(tcx);
        reads.visit_statement(statement, Location::START);
        self.reduce_reads(tcx, decls, reads)
    }

    pub(super) fn demand_terminator(
        &mut self,
        tcx: TyCtxt<'tcx>,
        decls: &LocalDecls<'tcx>,
        term: &Terminator<'tcx>,
    ) -> bool {
        let mut reads = ReadPlaces::new(tcx);
        reads.visit_terminator(term, Location::START);
        if matches!(term.kind, rustc_middle::mir::TerminatorKind::Return) {
            reads.push(Place::from(rustc_middle::mir::RETURN_PLACE));
        }
        self.reduce_reads(tcx, decls, reads)
    }
    fn reduce_reads(
        &mut self,
        tcx: TyCtxt<'tcx>,
        decls: &LocalDecls<'tcx>,
        reads: ReadPlaces<'tcx>,
    ) -> bool {
        if !self.feasible {
            return false;
        }
        for index in reads.indices {
            if !self.reduce_place(tcx, decls, index) {
                return false;
            }
        }
        reads.places.into_iter().all(|place| {
            let place = resolve_place(tcx, self, place).unwrap_or(place);
            self.reduce_place(tcx, decls, place)
        })
    }
}

struct ReadPlaces<'tcx> {
    tcx: TyCtxt<'tcx>,
    places: Vec<Place<'tcx>>,
    indices: Vec<Place<'tcx>>,
    seen: HashSet<Place<'tcx>>,
}

impl<'tcx> ReadPlaces<'tcx> {
    fn new(tcx: TyCtxt<'tcx>) -> Self {
        Self {
            tcx,
            places: Vec::new(),
            indices: Vec::new(),
            seen: HashSet::new(),
        }
    }
    fn push(&mut self, place: Place<'tcx>) {
        if self.seen.insert(place) {
            self.places.push(place);
        }
    }
}

impl<'tcx> Visitor<'tcx> for ReadPlaces<'tcx> {
    fn visit_place(&mut self, place: &Place<'tcx>, context: PlaceContext, location: Location) {
        if matches!(context, PlaceContext::NonUse(_)) {
            return;
        }
        if !context.is_place_assignment() {
            self.push(*place);
        }
        for (base, elem) in place.iter_projections() {
            match elem {
                ProjectionElem::Index(local) => self.indices.push(Place::from(local)),
                ProjectionElem::Deref => self.push(base.to_place(self.tcx)),
                _ => {}
            }
        }
        self.super_place(place, context, location);
    }
}

pub(super) fn resolve_place<'tcx>(
    tcx: TyCtxt<'tcx>,
    state: &CombinedState<'tcx>,
    place: Place<'tcx>,
) -> Option<Place<'tcx>> {
    if !place
        .projection
        .iter()
        .any(|elem| matches!(elem, ProjectionElem::Index(_)))
    {
        return Some(place);
    }
    let mut projection = place.projection.to_vec();
    for elem in &mut projection {
        if let ProjectionElem::Index(local) = elem {
            let value = state
                .interval
                .tracked_interval_resolved(&state.symbolic, &Place::from(*local))?;
            if value.low != value.high {
                return None;
            }
            let offset = u64::try_from(value.low).ok()?;
            *elem = ProjectionElem::ConstantIndex {
                offset,
                min_length: offset.checked_add(1)?,
                from_end: false,
            };
        }
    }
    Some(Place {
        local: place.local,
        projection: tcx.mk_place_elems(&projection),
    })
}
