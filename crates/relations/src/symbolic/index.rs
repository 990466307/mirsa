use super::{SymbolicExpr, SymbolicFact, SymbolicState};
use mirsa_framework::access_path::AccessPath;
use mirsa_framework::revision::{Revision, Versioned};
use rustc_middle::mir::Place;
use std::collections::HashMap;

type Inputs = (Revision, Revision);

#[derive(Clone, Debug, Default)]
pub(super) struct IndexInputs {
    pub(super) expressions: Option<Inputs>,
    pub(super) facts: Option<Inputs>,
    pub(super) predicates: Option<Inputs>,
}

fn reusable<T: PartialEq>(
    indexed: Option<Inputs>,
    previous: &Versioned<T>,
    current: &Versioned<T>,
    old_targets: &Versioned<HashMap<AccessPath, AccessPath>>,
    targets: &Versioned<HashMap<AccessPath, AccessPath>>,
) -> bool {
    indexed == Some((previous.revision(), old_targets.revision()))
        && previous == current
        && old_targets == targets
}

impl<'tcx> SymbolicState<'tcx> {
    pub(super) fn reuse_or_rebuild_indexes(&mut self, sources: [&Self; 2]) {
        if let Some(source) = sources.iter().find(|s| {
            reusable(
                s.index_inputs.expressions,
                &s.exprs,
                &self.exprs,
                &s.points_to,
                &self.points_to,
            )
        }) {
            self.dependencies = source.dependencies.clone();
            self.dependents = source.dependents.clone();
        } else {
            let expressions = std::mem::take(&mut self.exprs);
            for (path, expr) in &expressions {
                self.index_expr(path, expr);
            }
            self.exprs = expressions;
        }
        self.index_inputs.expressions = Some((self.exprs.revision(), self.points_to.revision()));

        if let Some(source) = sources.iter().find(|s| {
            reusable(
                s.index_inputs.predicates,
                &s.predicates,
                &self.predicates,
                &s.points_to,
                &self.points_to,
            )
        }) {
            self.predicate_index = source.predicate_index.clone();
        } else {
            self.rebuild_predicate_index();
        }
        self.index_inputs.predicates =
            Some((self.predicates.revision(), self.points_to.revision()));

        if let Some(source) = sources.iter().find(|s| {
            reusable(
                s.index_inputs.facts,
                &s.facts,
                &self.facts,
                &s.points_to,
                &self.points_to,
            )
        }) {
            self.fact_index = source.fact_index.clone();
        } else {
            self.rebuild_value_fact_index();
        }
        self.index_inputs.facts = Some((self.facts.revision(), self.points_to.revision()));
    }

    pub(super) fn index_expr(&mut self, path: &AccessPath, expr: &SymbolicExpr<'tcx>) {
        let mut inputs = Vec::new();
        expr.visit_places(|place| {
            if let Some(input) = self.normalize_place(place) {
                self.display_places.insert_if_absent(input.clone(), place);
                if !inputs.contains(&input) {
                    inputs.push(input);
                }
            }
        });
        for input in &inputs {
            self.dependents
                .entry(input.clone())
                .or_default()
                .push(path.clone());
        }
        self.dependencies.insert(path.clone(), inputs);
    }

    pub(super) fn remove_expr(&mut self, path: &AccessPath) {
        self.exprs.remove(path);
        if let Some(inputs) = self.dependencies.remove(path) {
            for input in inputs {
                if let Some(outputs) = self.dependents.get_mut(&input) {
                    outputs.retain(|output| output != path);
                    if outputs.is_empty() {
                        self.dependents.remove(&input);
                    }
                }
            }
        }
    }

    pub(super) fn index_fact(&mut self, index: usize, fact: &SymbolicFact<'tcx>) {
        let (SymbolicFact::EqConst { expr, .. } | SymbolicFact::NeConst { expr, .. }) = fact;
        if let Some(place) = expr.place() {
            if let Some(path) = self.normalize_place(place) {
                self.display_places.insert_if_absent(path.clone(), place);
                self.fact_index.entry(path).or_default().push(index);
            }
        }
    }

    pub(super) fn rebuild_fact_index(&mut self) {
        self.rebuild_predicate_index();
        self.rebuild_value_fact_index();
    }

    fn rebuild_value_fact_index(&mut self) {
        self.fact_index.clear();
        for index in 0..self.facts.len() {
            let fact = self.facts[index].clone();
            self.index_fact(index, &fact);
        }
        self.index_inputs.facts = Some((self.facts.revision(), self.points_to.revision()));
    }

    pub fn equivalent_places(&self, place: Place<'tcx>, limit: usize) -> Vec<Place<'tcx>> {
        let Some(path) = self.normalize_place(place) else {
            return Vec::new();
        };
        self.eq
            .members(path)
            .take(limit)
            .filter_map(|path| self.place_for_path(&path))
            .collect()
    }

    pub fn related_places(&self, place: Place<'tcx>, limit: usize) -> Vec<Place<'tcx>> {
        let Some(path) = self.normalize_place(place) else {
            return Vec::new();
        };
        let expression = self.expression_path(&path).unwrap_or_else(|| path.clone());
        let mut out = Vec::new();
        for related in self
            .dependencies
            .get(&expression)
            .into_iter()
            .flatten()
            .take(limit)
            .cloned()
            .chain(
                self.dependents
                    .get(&path)
                    .into_iter()
                    .flatten()
                    .take(limit)
                    .cloned(),
            )
            .chain(self.eq.members(path).take(limit))
        {
            if out.len() == limit {
                break;
            }
            if let Some(input) = self.place_for_path(&related) {
                if !out.contains(&input) {
                    out.push(input);
                }
            }
        }
        out
    }

    pub fn facts_for_place(&self, place: Place<'tcx>, limit: usize) -> Vec<SymbolicFact<'tcx>> {
        let Some(path) = self.normalize_place(place) else {
            return Vec::new();
        };
        self.fact_index
            .get(&path)
            .into_iter()
            .flatten()
            .take(limit)
            .map(|index| self.facts[*index].clone())
            .collect()
    }
}
