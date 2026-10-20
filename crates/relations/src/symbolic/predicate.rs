use super::SymbolicState;
use mirsa_framework::{access_path::AccessPath, mapping::PathMapping, reduction::MAX_VALUES};
use rustc_middle::mir::{Operand, Place};
use std::collections::BTreeMap;

/// A verified, pure value predicate. Its bindings contain no memory reads.
#[derive(Clone, Debug, PartialEq, Hash)]
pub struct PredicateFact<'tcx> {
    pub expression: String,
    pub values: BTreeMap<String, Operand<'tcx>>,
}
impl Eq for PredicateFact<'_> {}

impl<'tcx> PredicateFact<'tcx> {
    pub fn project(&self, map: &impl PathMapping<'tcx>) -> Option<Self> {
        Some(Self {
            expression: self.expression.clone(),
            values: self
                .values
                .iter()
                .map(|(name, value)| Some((name.clone(), map.operand(value)?)))
                .collect::<Option<_>>()?,
        })
    }
}

impl<'tcx> SymbolicState<'tcx> {
    pub fn add_predicate(&mut self, fact: PredicateFact<'tcx>) {
        if self.predicates.len() < MAX_VALUES && !self.predicates.contains(&fact) {
            self.predicates.push(fact);
            self.rebuild_predicate_index();
        }
    }

    pub(super) fn rebuild_predicate_index(&mut self) {
        self.predicate_index.clear();
        for (index, fact) in self.predicates.iter().enumerate() {
            for place in fact.values.values().filter_map(Operand::place) {
                if let Some(path) = self.normalize_place(place) {
                    self.display_places.entry(path.clone()).or_insert(place);
                    let ids = self.predicate_index.entry(path).or_default();
                    if !ids.contains(&index) {
                        ids.push(index);
                    }
                }
            }
        }
        self.index_inputs.predicates =
            Some((self.predicates.revision(), self.points_to.revision()));
    }

    pub fn predicates_for_place(
        &self,
        place: Place<'tcx>,
        limit: usize,
    ) -> Vec<PredicateFact<'tcx>> {
        let Some(path) = self.normalize_place(place) else {
            return Vec::new();
        };
        self.eq
            .members(path)
            .take(limit)
            .flat_map(|p| self.predicate_index.get(&p).into_iter().flatten())
            .take(limit)
            .map(|i| self.predicates[*i].clone())
            .collect()
    }

    pub(super) fn copied_predicates(
        &self,
        dst: Place<'tcx>,
        src: Place<'tcx>,
    ) -> Vec<PredicateFact<'tcx>> {
        if !dst.projection.is_empty() || !src.projection.is_empty() || dst == src {
            return Vec::new();
        }
        let written = AccessPath::from_place(dst).unwrap();
        self.predicates
            .iter()
            .filter_map(|fact| {
                // A relation about the old destination cannot survive its overwrite.
                if fact.values.values().filter_map(Operand::place).any(|p| {
                    AccessPath::from_place(p)
                        .is_some_and(|p| self.write_affects(&p, std::slice::from_ref(&written)))
                }) {
                    return None;
                }
                let mut copy = fact.clone();
                let mut changed = false;
                for value in copy.values.values_mut() {
                    if value.place() == Some(src) {
                        *value = Operand::Copy(dst);
                        changed = true;
                    }
                }
                changed.then_some(copy)
            })
            .collect()
    }
}
