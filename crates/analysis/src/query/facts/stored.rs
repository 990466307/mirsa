use super::super::{
    Diagnostic, FactValue,
    value::{Eval, Value},
};
use super::{expressions::Exporter, fact_value, model::*};
use mirsa_domains::{
    allocation::state::AllocationEntry,
    interval::{abstract_value::Interval, state::IntervalFact},
    nullptr::abstract_value::NullPtr,
};
use mirsa_framework::access_path::AccessPath;
use mirsa_relations::symbolic::{SymbolicEntry, SymbolicFact as NativeFact};
use std::collections::HashMap;

pub(super) type AbstractFacts = HashMap<Expression, FactValue>;
pub(super) fn range(value: &Interval) -> FactValue {
    FactValue::Integer {
        low: value.low.to_string(),
        high: value.high.to_string(),
    }
}

pub(super) fn export<'tcx>(
    eval: &Eval<'_, 'tcx>,
    roots: &[AccessPath],
    names: &Exporter<'_, 'tcx>,
) -> Result<(AbstractFacts, Vec<SymbolicFact>), Vec<Diagnostic>> {
    let mut abstract_info = HashMap::new();
    let mut symbolic_info = Vec::new();
    for fact in eval.state.interval.export_facts(roots) {
        let (subject, value) = match fact {
            IntervalFact::Range(p, v) if !v.is_empty() && *v != Interval::top() => {
                (names.path(p), range(v))
            }
            IntervalFact::Range(..) => continue,
            IntervalFact::Bits(p, v) => (
                names.path(p).property(Property::Bits),
                FactValue::Bits {
                    bits: v.to_bits_unchecked().to_string(),
                    width: v.size().bits(),
                },
            ),
            IntervalFact::Variant(p, v) => (
                names.path(p).property(Property::Variant),
                FactValue::Integer {
                    low: v.to_string(),
                    high: v.to_string(),
                },
            ),
            IntervalFact::Float(p, v) if !v.is_bottom() => {
                (names.path(p), fact_value(eval, Value::Float(*v)))
            }
            IntervalFact::Float(..) => continue,
            IntervalFact::Length(p, v) if !v.is_empty() && *v != Interval::top() => {
                (names.path(p).property(Property::Length), range(v))
            }
            IntervalFact::Length(..) => continue,
        };
        abstract_info.insert(subject, value);
    }
    for (path, v) in eval.state.nullptr.export_facts(roots) {
        let value = match v {
            NullPtr::Bot => continue,
            NullPtr::Null => FactValue::Bool { value: Some(true) },
            NullPtr::NonNull => FactValue::Bool { value: Some(false) },
            NullPtr::MaybeNull => FactValue::Bool { value: None },
        };
        abstract_info.insert(names.path(path).property(Property::IsNull), value);
    }
    for fact in eval.state.allocation.export_facts(roots) {
        match fact {
            AllocationEntry::Pointer(_, value) | AllocationEntry::Buffer(_, value)
                if value.is_bottom() =>
            {
                continue;
            }
            AllocationEntry::Layout(_, value) if value.is_bottom() => continue,
            AllocationEntry::Pointer(p, v) => {
                abstract_info.insert(names.path(p), fact_value(eval, Value::Pointer(v.clone())));
            }
            AllocationEntry::Buffer(p, v) => {
                abstract_info.insert(
                    names.path(p).property(Property::Buffer),
                    fact_value(eval, Value::Pointer(v.clone())),
                );
            }
            AllocationEntry::Layout(p, v) => {
                abstract_info.insert(names.path(p).property(Property::LayoutSize), range(&v.size));
                abstract_info.insert(
                    names.path(p).property(Property::LayoutAlign),
                    range(&v.align),
                );
            }
            AllocationEntry::Object(id, v) => {
                abstract_info.insert(
                    Expression::Allocation {
                        identity: id.to_string(),
                    },
                    FactValue::Allocation {
                        alignment: v.alignment,
                        origin: v.origin.to_string(),
                        extent_low: v.extent.low.to_string(),
                        extent_high: v.extent.high.to_string(),
                        may_be_absent: v.status.may_be_absent(),
                        may_be_live: v.status.may_be_live(),
                        may_be_dead: v.status.may_be_dead(),
                        may_be_zero: v.multiplicity.may_be_zero(),
                        may_be_one: v.multiplicity.may_be_one(),
                        may_be_many: v.multiplicity.may_be_many(),
                    },
                );
            }
        }
    }
    for fact in eval.state.symbolic.export_facts(roots) {
        match fact {
            SymbolicEntry::Expression(p, e) => symbolic_info.push(SymbolicFact::Definition {
                target: names.path(p),
                expression: names.symbolic(e),
            }),
            SymbolicEntry::Condition(fact) => {
                let (operand, bits, operator) = match fact {
                    NativeFact::EqConst { expr, value } => (expr, value, BinaryOperator::Eq),
                    NativeFact::NeConst { expr, value } => (expr, value, BinaryOperator::Ne),
                };
                symbolic_info.push(SymbolicFact::Condition {
                    expression: Expression::Binary {
                        operator,
                        left: Box::new(names.operand(operand)),
                        right: Box::new(
                            names.constant(*bits, operand.ty(&eval.body.local_decls, eval.tcx)),
                        ),
                    },
                });
            }
            SymbolicEntry::Predicate(p) => symbolic_info.push(SymbolicFact::Condition {
                expression: names.predicate(p)?,
            }),
            SymbolicEntry::Equality(members) => {
                let mut members: Vec<_> = members.iter().map(|p| names.path(p)).collect();
                members.sort_by_cached_key(|p| format!("{p:?}"));
                members.dedup();
                if members.len() > 1 {
                    symbolic_info.push(SymbolicFact::SameValue { members });
                }
            }
            SymbolicEntry::PointsTo(p, t) => symbolic_info.push(SymbolicFact::PointsTo {
                pointer: names.path(p),
                target: names.path(t),
            }),
            SymbolicEntry::AddressTaken(p) => {
                abstract_info.insert(
                    names.path(p).property(Property::MayHaveAddressTaken),
                    FactValue::Bool { value: Some(true) },
                );
            }
            SymbolicEntry::Escaped(p) => {
                abstract_info.insert(
                    names.path(p).property(Property::MayEscape),
                    FactValue::Bool { value: Some(true) },
                );
            }
        }
    }
    symbolic_info.sort_by_cached_key(|fact| format!("{fact:?}"));
    symbolic_info.dedup();
    Ok((abstract_info, symbolic_info))
}
