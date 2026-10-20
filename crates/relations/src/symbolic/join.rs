use super::SymbolicState;
use mirsa_framework::eq_domain::join_eq;
use std::collections::{HashMap, HashSet};
use std::hash::Hash;

fn intersect<T: Clone + Eq + Hash>(left: &[T], right: &[T]) -> Vec<T> {
    if left.is_empty() || right.is_empty() {
        return Vec::new();
    }
    let membership: HashSet<_> = right.iter().collect();
    // Preserve the left-hand order: bounded consumers observe this order.
    left.iter()
        .filter(|value| membership.contains(value))
        .cloned()
        .collect()
}

impl<'tcx> SymbolicState<'tcx> {
    pub fn join(left: &Self, right: &Self) -> Self {
        let mut out = Self {
            eq: join_eq(&left.eq, &right.eq),
            display_places: left.display_places.clone(),
            exprs: left
                .exprs
                .iter()
                .filter(|(path, expr)| right.exprs.get(*path) == Some(*expr))
                .map(|(path, expr)| (path.clone(), expr.clone()))
                .collect(),
            predicates: intersect(&left.predicates, &right.predicates).into(),
            predicate_index: HashMap::new().into(),
            facts: intersect(&left.facts, &right.facts).into(),
            dependents: HashMap::new().into(),
            dependencies: HashMap::new().into(),
            fact_index: HashMap::new().into(),
            points_to: left
                .points_to
                .iter()
                .filter(|(path, pointee)| right.points_to.get(*path) == Some(*pointee))
                .map(|(path, pointee)| (path.clone(), pointee.clone()))
                .collect(),
            address_taken: left
                .address_taken
                .union(&right.address_taken)
                .cloned()
                .collect(),
            escaped: left.escaped.union(&right.escaped).cloned().collect(),
            index_inputs: Default::default(),
            debug: left.debug || right.debug,
        };
        out.merge_display_places_from(right);
        out.reuse_or_rebuild_indexes([left, right]);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::symbolic::{PredicateFact, SymbolicExpr, SymbolicFact};
    use mirsa_framework::access_path::AccessPath;
    use rustc_middle::mir::{Local, Operand, Place};

    #[derive(Clone, Debug, Eq, PartialEq)]
    struct Colliding(u8);
    impl Hash for Colliding {
        fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
            0u8.hash(state);
        }
    }

    #[test]
    fn intersection_preserves_order_and_checks_equality_after_hashing() {
        let left = [Colliding(3), Colliding(1), Colliding(2), Colliding(3)];
        let right = [Colliding(2), Colliding(3)];
        assert_eq!(
            intersect(&left, &right),
            vec![Colliding(3), Colliding(2), Colliding(3)]
        );
    }

    #[test]
    fn index_reuse_requires_unchanged_facts_and_alias_context() {
        let place = |n| Place::from(Local::from_usize(n));
        let mut seed = SymbolicState::new();
        seed.set_place_expr(place(0), SymbolicExpr::Length { place: place(1) });
        seed.assume_eq_const(Operand::Copy(place(1)), 8);
        seed.assume_eq_const(Operand::Copy(place(2)), 16);
        seed.add_predicate(PredicateFact {
            expression: "$x > 0".into(),
            values: [("x".into(), Operand::Copy(place(1)))].into(),
        });
        let indexed = SymbolicState::join(&seed, &seed);
        let reused = SymbolicState::join(&indexed, &indexed);
        assert_eq!(
            reused.dependencies.revision(),
            indexed.dependencies.revision()
        );
        assert_eq!(reused.fact_index.revision(), indexed.fact_index.revision());
        assert_eq!(
            reused.predicate_index.revision(),
            indexed.predicate_index.revision()
        );

        let mut changed = indexed.clone();
        changed.facts.remove(0);
        let joined = SymbolicState::join(&indexed, &changed);
        assert!(joined.facts_for_place(place(1), 8).is_empty());
        assert_eq!(
            joined.facts_for_place(place(2), 8),
            vec![SymbolicFact::EqConst {
                expr: Operand::Copy(place(2)),
                value: 16
            }]
        );

        // Even if the selected relations are unchanged, alias writes invalidate
        // the normalization context under which their indexes were built.
        changed = indexed.clone();
        changed.set_points_to(
            AccessPath::from_local(Local::from_usize(1)),
            AccessPath::from_local(Local::from_usize(2)),
        );
        let refreshed = SymbolicState::join(&changed, &changed);
        assert_ne!(
            refreshed.dependencies.revision(),
            indexed.dependencies.revision()
        );
        assert_ne!(
            refreshed.fact_index.revision(),
            indexed.fact_index.revision()
        );
        assert_ne!(
            refreshed.predicate_index.revision(),
            indexed.predicate_index.revision()
        );
    }
}
