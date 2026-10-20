#[cfg(feature = "z3")]
mod backend;
mod formula;

pub use formula::{BinaryOp, Comparison, FloatTerm, Formula, IntTerm, ScalarType, Term, UnaryOp};
use std::time::Duration;

#[derive(Clone, Debug)]
pub struct Problem {
    pub assumptions: Vec<Formula>,
    pub property: Formula,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
pub enum Verdict {
    Proven,
    Refuted,
    Unknown,
    Unreachable,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
pub enum UnknownReason {
    BackendDisabled,
    Unsupported,
    BudgetExceeded,
    SolverUnknown,
    InsufficientInformation,
    MixedPaths,
}

#[derive(Clone, Debug)]
pub struct SolveResult {
    pub verdict: Verdict,
    pub reason: Option<UnknownReason>,
    pub candidate: Option<String>,
}

impl SolveResult {
    pub fn known(verdict: Verdict) -> Self {
        Self {
            verdict,
            reason: None,
            candidate: None,
        }
    }

    pub fn unknown(reason: UnknownReason) -> Self {
        Self {
            verdict: Verdict::Unknown,
            reason: Some(reason),
            candidate: None,
        }
    }
}

pub fn solve(problem: &Problem, timeout: Duration) -> SolveResult {
    if timeout.is_zero() || !within_budget(problem) {
        return SolveResult::unknown(UnknownReason::BudgetExceeded);
    }
    #[cfg(feature = "z3")]
    {
        backend::solve(problem, timeout)
    }
    #[cfg(not(feature = "z3"))]
    {
        let _ = problem;
        SolveResult::unknown(UnknownReason::BackendDisabled)
    }
}

fn within_budget(problem: &Problem) -> bool {
    enum Node<'a> {
        Formula(&'a Formula),
        Term(&'a Term),
        Integer(&'a IntTerm),
        Float(&'a FloatTerm),
    }
    if problem.assumptions.len() > 4096 {
        return false;
    }
    let mut pending: Vec<_> = problem
        .assumptions
        .iter()
        .chain(std::iter::once(&problem.property))
        .map(|formula| (Node::Formula(formula), 0))
        .collect();
    let mut count = 0;
    while let Some((node, depth)) = pending.pop() {
        count += 1;
        if count > 4096 || depth > 128 {
            return false;
        }
        match node {
            Node::Formula(formula) => match formula {
                Formula::FloatCompare(_, a, b) | Formula::FloatSame(a, b) => {
                    pending.push((Node::Float(a), depth + 1));
                    pending.push((Node::Float(b), depth + 1));
                }
                Formula::IsNan(a) | Formula::IsInfinite(a) => {
                    pending.push((Node::Float(a), depth + 1))
                }
                Formula::MathCompare(_, a, b) => {
                    pending.push((Node::Integer(a), depth + 1));
                    pending.push((Node::Integer(b), depth + 1));
                }
                Formula::Compare(_, a, b) => {
                    pending.push((Node::Term(a), depth + 1));
                    pending.push((Node::Term(b), depth + 1));
                }
                Formula::Not(value) => pending.push((Node::Formula(value), depth + 1)),
                Formula::All(values) | Formula::Any(values) => {
                    if values.len() > 4096 {
                        return false;
                    }
                    pending.extend(values.iter().map(|value| (Node::Formula(value), depth + 1)));
                }
                Formula::Literal(_) => {}
            },
            Node::Integer(term) => match term {
                IntTerm::Variable(_) => {}
                IntTerm::Constant(n) => {
                    if n.len() > 4096 {
                        return false;
                    }
                }
                IntTerm::FromBits(t) => pending.push((Node::Term(t), depth + 1)),
                IntTerm::Binary(_, a, b) | IntTerm::Bitwise(_, a, b, _) => {
                    if matches!(term, IntTerm::Bitwise(_, _, _, bits) if *bits == 0 || *bits > 8193)
                    {
                        return false;
                    }
                    pending.push((Node::Integer(a), depth + 1));
                    pending.push((Node::Integer(b), depth + 1));
                }
                IntTerm::Neg(t) => pending.push((Node::Integer(t), depth + 1)),
            },
            Node::Term(term) => match term {
                Term::FromInteger(value, _) => pending.push((Node::Integer(value), depth + 1)),
                Term::FloatBits(value) | Term::FloatToInteger(value, _) => {
                    pending.push((Node::Float(value), depth + 1))
                }
                Term::Binary(_, a, b) | Term::Overflow(_, a, b) => {
                    pending.push((Node::Term(a), depth + 1));
                    pending.push((Node::Term(b), depth + 1));
                }
                Term::Unary(_, value) | Term::Cast(value, _) => {
                    pending.push((Node::Term(value), depth + 1))
                }
                Term::Select(_, index, _) => pending.push((Node::Term(index), depth + 1)),
                Term::Boolean(value) => pending.push((Node::Formula(value), depth + 1)),
                _ => {}
            },
            Node::Float(term) => match term {
                FloatTerm::FromMath(t, _) => pending.push((Node::Integer(t), depth + 1)),
                FloatTerm::FromBits(t) | FloatTerm::FromInteger(t, _) => {
                    pending.push((Node::Term(t), depth + 1))
                }
                FloatTerm::Cast(t, _) | FloatTerm::Neg(t) => {
                    pending.push((Node::Float(t), depth + 1))
                }
                FloatTerm::Binary(_, a, b) => {
                    pending.push((Node::Float(a), depth + 1));
                    pending.push((Node::Float(b), depth + 1));
                }
            },
        }
    }
    true
}
