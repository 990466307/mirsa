//! Owned evidence from the same abstract state and formula used for a check.
use super::facts::expressions::Exporter;
use super::value::{Eval, TypedKind};
use super::*;
use crate::reduced_product::CombinedState;
use mirsa_framework::access_path::AccessPath;
use serde::Serialize;
mod expressions;
pub use expressions::ExpressionEvidence;

#[derive(Clone, Debug, Serialize)]
pub struct Evidence {
    /// Compact expression view for consumers; detailed records remain below.
    pub expressions: Option<ExpressionEvidence>,
    /// Abstract evaluation before any solver fallback. None when the budget prevented evaluation.
    pub abstract_evaluation: Option<Evaluation>,
    /// Relevant stored facts, not a minimal set of premises or source provenance.
    pub context: Vec<SymbolicFact>,
    pub smt: Option<SolverEvidence>,
}

#[derive(Clone, Debug, Serialize)]
pub struct SolverEvidence {
    /// Exact typed formula submitted to mirsa-solver.
    pub problem: mirsa_solver::Problem,
    pub bindings: Vec<SolverBinding>,
    /// Actual Z3 assertions and both checks; absent if encoding/backend failed.
    pub checks: Option<mirsa_solver::SmtEvidence>,
}

#[derive(Clone, Debug, Serialize)]
pub struct SolverBinding {
    pub variable: u32,
    pub subject: Expression,
    pub scalar_type: mirsa_solver::ScalarType,
}

#[derive(Clone, Debug, Serialize)]
pub struct Evaluation {
    pub expression: Expression,
    pub rule: EvaluationRule,
    pub value: FactValue,
    pub premises: Vec<Evaluation>,
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EvaluationRule {
    AbstractValue,
    Constant,
    Length,
    Builtin,
    Projection,
    Cast,
    Arithmetic,
    Comparison,
    EquivalentValues,
    BooleanValue,
    Not,
    All,
    Any,
}

pub(super) fn collect<'tcx>(
    tcx: TyCtxt<'tcx>,
    body: &Body<'tcx>,
    state: &CombinedState<'tcx>,
    property: &Predicate<'tcx>,
    bindings: &Bindings<'tcx>,
) -> Evidence {
    let names = Exporter::new(tcx, body, Some(state), bindings);
    let eval = Eval { tcx, body, state };
    let mut roots = Vec::new();
    property.visit_places(&mut |p| {
        if let Some(path) = AccessPath::from_place(p) {
            roots.push(path);
        }
    });
    let roots = state.symbolic.fact_roots(&roots);
    let context = super::facts::stored::export(&eval, &roots, &names)
        .map(|(_, symbolic)| symbolic)
        .unwrap_or_default();
    Evidence {
        expressions: None,
        abstract_evaluation: Some(predicate(&eval, &names, property)),
        context,
        smt: None,
    }
}

fn binary(operator: BinaryOperator, left: Expression, right: Expression) -> Expression {
    Expression::Binary {
        operator,
        left: Box::new(left),
        right: Box::new(right),
    }
}
fn unary(operator: UnaryOperator, operand: Expression) -> Expression {
    Expression::Unary {
        operator,
        operand: Box::new(operand),
    }
}

fn predicate<'tcx>(
    eval: &Eval<'_, 'tcx>,
    names: &Exporter<'_, 'tcx>,
    p: &Predicate<'tcx>,
) -> Evaluation {
    use EvaluationRule as R;
    let (expression, rule, premises) = match p {
        Predicate::Value(e) => {
            let child = expression(eval, names, e);
            (child.expression.clone(), R::BooleanValue, vec![child])
        }
        Predicate::Compare(op, a, b) => {
            let left = expression(eval, names, a);
            let right = expression(eval, names, b);
            let operator = match op {
                Comparison::Eq => BinaryOperator::Eq,
                Comparison::Ne => BinaryOperator::Ne,
                Comparison::Lt => BinaryOperator::Lt,
                Comparison::Le => BinaryOperator::Le,
                Comparison::Gt => BinaryOperator::Gt,
                Comparison::Ge => BinaryOperator::Ge,
            };
            let result = abstract_query::evaluate(eval.tcx, eval.body, eval.state, p);
            let rule = if result.is_some() && eval.compare(*op, a, b).is_none() {
                R::EquivalentValues
            } else {
                R::Comparison
            };
            (
                binary(operator, left.expression.clone(), right.expression.clone()),
                rule,
                vec![left, right],
            )
        }
        Predicate::Not(p) => {
            let child = predicate(eval, names, p);
            (
                unary(UnaryOperator::Not, child.expression.clone()),
                R::Not,
                vec![child],
            )
        }
        Predicate::All(ps) | Predicate::Any(ps) => {
            let all = matches!(p, Predicate::All(_));
            let mut children = Vec::new();
            // Match the evaluator's short circuit, including unknown operands.
            for p in ps {
                let child = predicate(eval, names, p);
                let stop = matches!(child.value, FactValue::Bool { value: Some(v) } if v != all);
                children.push(child);
                if stop {
                    break;
                }
            }
            let e = Expression::Call {
                callee: if all { "all" } else { "any" }.into(),
                arguments: children.iter().map(|c| c.expression.clone()).collect(),
            };
            (e, if all { R::All } else { R::Any }, children)
        }
    };
    Evaluation {
        expression,
        rule,
        premises,
        value: FactValue::Bool {
            value: abstract_query::evaluate(eval.tcx, eval.body, eval.state, p),
        },
    }
}

fn expression<'tcx>(
    eval: &Eval<'_, 'tcx>,
    names: &Exporter<'_, 'tcx>,
    e: &Expr<'tcx>,
) -> Evaluation {
    use EvaluationRule as R;
    let mut premises = Vec::new();
    let (subject, rule) = match e {
        Expr::Value(v) => (names.operand(v), R::AbstractValue),
        Expr::Constant { bits, ty } => (names.constant(*bits, *ty), R::Constant),
        Expr::Length(p) => (
            names
                .operand(&rustc_middle::mir::Operand::Copy(*p))
                .property(Property::Length),
            R::Length,
        ),
        Expr::TypedExpr(v) => match &v.kind {
            TypedKind::Boolean(p) => {
                let child = predicate(eval, names, p);
                let subject = child.expression.clone();
                premises.push(child);
                (subject, R::BooleanValue)
            }
            TypedKind::Integer(n) => (
                Expression::Literal {
                    token: n.to_string(),
                },
                R::Constant,
            ),
            TypedKind::Float(n) => (
                Expression::Literal {
                    token: n.to_string(),
                },
                R::Constant,
            ),
            TypedKind::Builtin(op, args) => {
                premises = args.iter().map(|e| expression(eval, names, e)).collect();
                let args: Vec<_> = premises.iter().map(|p| p.expression.clone()).collect();
                let subject = if let (Some(property), [argument]) =
                    (super::facts::expressions::property(*op), args.as_slice())
                {
                    argument.clone().property(property)
                } else {
                    Expression::Call {
                        callee: format!("{op:?}"),
                        arguments: args,
                    }
                };
                (subject, R::Builtin)
            }
            TypedKind::Project { .. } => {
                premises = v
                    .children()
                    .iter()
                    .map(|e| expression(eval, names, e))
                    .collect();
                let subject = eval
                    .place(e)
                    .map(|p| names.operand(&rustc_middle::mir::Operand::Copy(p)))
                    .unwrap_or_else(|| Expression::Call {
                        callee: "projection".into(),
                        arguments: premises.iter().map(|p| p.expression.clone()).collect(),
                    });
                (subject, R::Projection)
            }
        },
        Expr::Binary(op, a, b) => {
            premises = vec![expression(eval, names, a), expression(eval, names, b)];
            let op = match op {
                BinaryOp::Add => BinaryOperator::Add,
                BinaryOp::Sub => BinaryOperator::Sub,
                BinaryOp::Mul => BinaryOperator::Mul,
                BinaryOp::Div => BinaryOperator::Div,
                BinaryOp::Rem => BinaryOperator::Rem,
                BinaryOp::And => BinaryOperator::BitAnd,
                BinaryOp::Or => BinaryOperator::BitOr,
                BinaryOp::Xor => BinaryOperator::BitXor,
                BinaryOp::Shl => BinaryOperator::Shl,
                BinaryOp::Shr => BinaryOperator::Shr,
            };
            (
                binary(
                    op,
                    premises[0].expression.clone(),
                    premises[1].expression.clone(),
                ),
                R::Arithmetic,
            )
        }
        Expr::Unary(op, a) => {
            let a = expression(eval, names, a);
            let e = unary(
                match op {
                    UnaryOp::Not => UnaryOperator::Not,
                    UnaryOp::Neg => UnaryOperator::Neg,
                },
                a.expression.clone(),
            );
            premises.push(a);
            (e, R::Arithmetic)
        }
        Expr::Cast(a, ty) => {
            let a = expression(eval, names, a);
            let e = Expression::Cast {
                cast_kind: "query".into(),
                ty: ty.to_string(),
                operand: Box::new(a.expression.clone()),
            };
            premises.push(a);
            (e, R::Cast)
        }
    };
    Evaluation {
        expression: subject,
        rule,
        value: super::facts::fact_value(eval, eval.eval(e)),
        premises,
    }
}
