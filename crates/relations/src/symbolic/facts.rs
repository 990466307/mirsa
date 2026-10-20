use super::{PredicateFact, SymbolicExpr, SymbolicFact, SymbolicState};
use mirsa_framework::{access_path::AccessPath, facts::selected};
use std::collections::HashSet;

pub enum SymbolicEntry<'a, 'tcx> {
    Expression(&'a AccessPath, &'a SymbolicExpr<'tcx>),
    Condition(&'a SymbolicFact<'tcx>),
    Predicate(&'a PredicateFact<'tcx>),
    Equality(Vec<AccessPath>),
    PointsTo(&'a AccessPath, &'a AccessPath),
    AddressTaken(&'a AccessPath),
    Escaped(&'a AccessPath),
}

impl<'tcx> SymbolicState<'tcx> {
    /// Expand aliases, not the transitive expression dependency graph.
    pub fn fact_roots(&self, roots: &[AccessPath]) -> Vec<AccessPath> {
        let mut out: HashSet<_> = roots.iter().cloned().collect();
        for root in roots {
            out.extend(self.eq.members(self.normalize_path(root)));
        }
        let normalized: Vec<_> = out.iter().cloned().collect();
        for path in self.eq.keys() {
            if selected(&path, &normalized) {
                out.extend(self.eq.members(path));
            }
        }
        out.into_iter().collect()
    }

    pub fn export_facts(&self, roots: &[AccessPath]) -> Vec<SymbolicEntry<'_, 'tcx>> {
        let mut out = Vec::new();
        let mut attached = roots.to_vec();
        for (path, expr) in &self.exprs {
            let mut inputs = Vec::new();
            expr.visit_places(|place| {
                if let Some(input) = self.normalize_place(place) {
                    inputs.push(input);
                }
            });
            if !selected(path, roots) && !inputs.iter().any(|p| selected(p, roots)) {
                continue;
            }
            out.push(SymbolicEntry::Expression(path, expr));
            attached.extend(self.eq.members(path.clone()));
        }
        for fact in &self.facts {
            let (SymbolicFact::EqConst { expr, .. } | SymbolicFact::NeConst { expr, .. }) = fact;
            if let Some(path) = expr.place().and_then(|p| self.normalize_place(p)) {
                if selected(&path, &attached) {
                    out.push(SymbolicEntry::Condition(fact));
                }
            }
        }
        for predicate in &self.predicates {
            let paths: Vec<_> = predicate
                .values
                .values()
                .filter_map(|v| v.place())
                .filter_map(|p| self.normalize_place(p))
                .collect();
            if paths.iter().any(|p| selected(p, &attached)) {
                out.push(SymbolicEntry::Predicate(predicate));
            }
        }
        let mut emitted = HashSet::new();
        for path in self.eq.keys() {
            if !selected(&path, roots) || emitted.contains(&path) {
                continue;
            }
            let members: Vec<_> = self.eq.members(path).collect();
            emitted.extend(members.iter().cloned());
            out.push(SymbolicEntry::Equality(members));
        }
        for (pointer, target) in &self.points_to {
            if selected(pointer, roots) || selected(target, roots) {
                out.push(SymbolicEntry::PointsTo(pointer, target));
            }
        }
        for (kind, paths) in [
            ("symbolic.address_taken", &self.address_taken),
            ("symbolic.escaped", &self.escaped),
        ] {
            for path in paths {
                if selected(path, roots) {
                    out.push(if kind == "symbolic.address_taken" {
                        SymbolicEntry::AddressTaken(path)
                    } else {
                        SymbolicEntry::Escaped(path)
                    });
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{symbolic::SymbolicExpr, value_expr::ValueExpr};
    use rustc_middle::mir::{BinOp, Local, Operand, Place};

    #[test]
    fn export_all_direct_relations_without_following_unrelated_dependencies() {
        let mut state = SymbolicState::new();
        let input = Place::from(Local::from_usize(1));
        let other = Place::from(Local::from_usize(2));
        for index in 3..43 {
            state.set_place_expr(
                Place::from(Local::from_usize(index)),
                SymbolicExpr::Value(ValueExpr::Binary {
                    op: BinOp::Div,
                    left: Operand::Copy(input),
                    right: Operand::Copy(other),
                }),
            );
        }
        state.set_place_expr(
            other,
            SymbolicExpr::Value(ValueExpr::Use(Operand::Copy(Place::from(
                Local::from_usize(100),
            )))),
        );
        let roots = state.fact_roots(&[AccessPath::from_place(input).unwrap()]);
        let facts = state.export_facts(&roots);
        assert_eq!(facts.len(), 40);
        assert!(facts.iter().all(|f| matches!(f, SymbolicEntry::Expression(path, SymbolicExpr::Value(ValueExpr::Binary { op: BinOp::Div, .. })) if path.root != other.local)));
    }
}
