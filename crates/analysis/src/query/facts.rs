mod display;
mod expressions;
pub mod model;
mod stored;

use super::value::{self, Eval, Value};
use super::{
    Bindings, Diagnostic, Expr, FactOutput, FactValue, PointerTarget, Predicate, QueryView,
};
use crate::reduced_product::CombinedState;
use expressions::Exporter;
use mirsa_dsl::Builtin;
use mirsa_framework::{access_path::AccessPath, forward::DomainState};
use mirsa_solver::UnknownReason;
use model::AbstractFact;
use std::time::Instant;

const FACT_BUILTINS: &[(&str, Builtin)] = &[
    ("len", Builtin::Len),
    ("is_nan", Builtin::IsNan),
    ("is_infinite", Builtin::IsInfinite),
    ("is_null", Builtin::IsNull),
    ("is_live", Builtin::IsLive),
    ("is_freed", Builtin::IsFreed),
    ("base", Builtin::Base),
    ("address", Builtin::Address),
    ("offset", Builtin::Offset),
    ("extent", Builtin::Extent),
    ("layout_size", Builtin::LayoutSize),
    ("layout_align", Builtin::LayoutAlign),
    ("discriminant", Builtin::Discriminant),
];

impl<'tcx> QueryView<'_, 'tcx> {
    pub(super) fn facts(
        &self,
        expr: &Expr<'tcx>,
        syntax: &mirsa_dsl::Expr,
        bindings: &Bindings<'tcx>,
    ) -> Result<FactOutput, Vec<Diagnostic>> {
        let started = Instant::now();
        let mut states = self.states.iter().map(|(_, _, state)| state);
        let mut merged = states.next().map(|first| {
            states.fold(first.clone(), |left, right| {
                CombinedState::join(&left, right)
            })
        });
        let names = Exporter::new(self.tcx, self.body, merged.as_ref(), bindings);
        let subject = names.syntax(syntax, &|name| {
            bindings.values.get(name).map(|v| {
                if let Some(path) = v.place().and_then(AccessPath::from_place) {
                    names.path(&path)
                } else {
                    names.operand(v)
                }
            })
        })?;
        let Some(state) = merged.as_mut() else {
            // No compiler values need resolving for an unreachable result.
            return Ok(FactOutput {
                abstract_info: vec![AbstractFact {
                    subject,
                    value: FactValue::Unreachable,
                }],
                symbolic_info: vec![],
            });
        };
        let property = Predicate::Value(expr.clone());
        let within_budget = property.within_budget(self.config.max_nodes);
        let exhausted = || !within_budget || started.elapsed() >= self.config.total_timeout;
        if !exhausted() {
            value::reduce(self.tcx, self.body, state, &property);
        }
        let mut roots = Vec::new();
        expr.visit_places(&mut |p| {
            if let Some(path) = AccessPath::from_place(p) {
                roots.push(path);
            }
        });
        let roots = state.symbolic.fact_roots(&roots);
        let eval = Eval {
            tcx: self.tcx,
            body: self.body,
            state,
        };
        let (mut abstract_info, symbolic_info) = stored::export(&eval, &roots, &names)?;
        let read = |expr: &Expr<'tcx>| {
            if exhausted() {
                FactValue::Unknown {
                    reason: UnknownReason::BudgetExceeded,
                }
            } else {
                fact_value(&eval, eval.eval(expr))
            }
        };
        let mut put = |subject, value| {
            if !matches!(value, FactValue::Unknown { .. }) || !abstract_info.contains_key(&subject)
            {
                abstract_info.insert(subject, value);
            }
        };
        put(subject.clone(), read(expr));
        for (_, op) in FACT_BUILTINS {
            if let Ok(value) = value::builtin(self.tcx, self.body, *op, vec![expr.clone()]) {
                put(
                    subject
                        .clone()
                        .property(expressions::property(*op).unwrap()),
                    read(&value),
                );
            }
        }
        let mut abstract_info = abstract_info
            .into_iter()
            .map(|(subject, value)| AbstractFact { subject, value })
            .collect::<Vec<_>>();
        abstract_info.sort_by_cached_key(|fact| format!("{:?}", fact.subject));
        Ok(FactOutput {
            abstract_info,
            symbolic_info,
        })
    }
}

fn fact_value(eval: &Eval<'_, '_>, value: Value) -> FactValue {
    match value {
        Value::Integer(a, b) => FactValue::Integer {
            low: a.to_string(),
            high: b.to_string(),
        },
        Value::Float(f) => FactValue::Float {
            low: f.low.to_string(),
            high: f.high.to_string(),
            may_nan: f.may_nan,
        },
        Value::Bool(value) => FactValue::Bool { value },
        Value::Pointer(p) => {
            let mut targets = p
                .targets()
                .map(|(id, offset)| {
                    let fact = eval.state.allocation.object(id);
                    PointerTarget {
                        allocation: id.to_string(),
                        offset_low: offset.low.to_string(),
                        offset_high: offset.high.to_string(),
                        extent_low: fact.map(|f| f.extent.low.to_string()),
                        extent_high: fact.map(|f| f.extent.high.to_string()),
                        status: fact.map(|f| f.status.to_string()),
                    }
                })
                .collect::<Vec<_>>();
            targets.sort_by(|a, b| a.allocation.cmp(&b.allocation));
            FactValue::Pointer {
                may_be_null: p.may_be_null(),
                may_be_invalid: p.may_be_invalid_non_null(),
                unknown_target: p.has_unknown_target(),
                targets,
            }
        }
        Value::Unknown => FactValue::Unknown {
            reason: UnknownReason::InsufficientInformation,
        },
    }
}
