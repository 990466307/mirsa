//! Human-readable expression projection; original typed evidence remains authoritative.
use super::*;
use mirsa_solver::{
    BinaryOp as B, Comparison as C, FloatTerm, Formula, IntTerm, ScalarType, Term, UnaryOp as U,
};
use std::collections::{BTreeSet, HashMap};

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ExpressionEvidence {
    Abstract {
        facts: Vec<String>,
        goal: String,
        result: Option<bool>,
    },
    Smt {
        variables: Vec<String>,
        /// All input assumptions, in order, including duplicates and tautologies.
        assumptions: Vec<String>,
        goal: String,
        negative: Option<mirsa_solver::Satisfiability>,
        positive: Option<mirsa_solver::Satisfiability>,
    },
}

fn name(e: &Expression) -> String {
    e.to_string().replace('$', "")
}
fn value(v: &FactValue) -> Option<String> {
    match v {
        FactValue::Integer { low, high } if low == high => Some(format!("= {low}")),
        FactValue::Integer { low, high } => Some(format!("in [{low}, {high}]")),
        FactValue::Bool { value: Some(b) } => Some(format!("= {b}")),
        FactValue::Float { low, high, may_nan } => {
            Some(format!("in [{low}, {high}], may_nan={may_nan}"))
        }
        _ => None,
    }
}

impl Evidence {
    pub(in crate::query) fn expression_view(&self) -> Option<ExpressionEvidence> {
        if let Some(smt) = &self.smt {
            let mut renderer = Renderer {
                bindings: smt.bindings.iter().map(|b| (b.variable, b)).collect(),
                used: BTreeSet::new(),
            };
            let assumptions = smt
                .problem
                .assumptions
                .iter()
                .map(|p| renderer.formula(p))
                .collect();
            let goal = renderer.formula(&smt.problem.property);
            let variables = renderer
                .used
                .iter()
                .map(|id| match renderer.bindings.get(id) {
                    Some(b) => format!("{}: {}", name(&b.subject), scalar(b.scalar_type)),
                    None => format!("v{id}"),
                })
                .collect();
            return Some(ExpressionEvidence::Smt {
                variables,
                assumptions,
                goal,
                negative: smt.checks.as_ref().map(|c| c.negative),
                positive: smt.checks.as_ref().and_then(|c| c.positive),
            });
        }
        let root = self.abstract_evaluation.as_ref()?;
        let mut facts = Vec::new();
        fn visit(node: &Evaluation, facts: &mut Vec<String>) {
            if !matches!(
                node.rule,
                EvaluationRule::BooleanValue
                    | EvaluationRule::Comparison
                    | EvaluationRule::EquivalentValues
                    | EvaluationRule::Not
                    | EvaluationRule::All
                    | EvaluationRule::Any
                    | EvaluationRule::Constant
            ) {
                if let Some(v) = value(&node.value) {
                    let fact = format!("{} {v}", name(&node.expression));
                    if !facts.contains(&fact) {
                        facts.push(fact);
                    }
                }
            }
            for child in &node.premises {
                visit(child, facts);
            }
        }
        visit(root, &mut facts);
        Some(ExpressionEvidence::Abstract {
            facts,
            goal: name(&root.expression),
            result: match root.value {
                FactValue::Bool { value } => value,
                _ => None,
            },
        })
    }
}

fn scalar(t: ScalarType) -> String {
    if t == ScalarType::BOOL {
        "bool".into()
    } else {
        format!("{}{}", if t.signed { "i" } else { "u" }, t.bits)
    }
}
fn cmp(op: C) -> &'static str {
    match op {
        C::Eq => "==",
        C::Ne => "!=",
        C::Lt => "<",
        C::Le => "<=",
        C::Gt => ">",
        C::Ge => ">=",
    }
}
fn op(op: B) -> &'static str {
    match op {
        B::Add => "+",
        B::Sub => "-",
        B::Mul => "*",
        B::Div => "/",
        B::Rem => "%",
        B::And => "&",
        B::Or => "|",
        B::Xor => "^",
        B::Shl => "<<",
        B::Shr => ">>",
    }
}
fn constant(bits: u128, ty: ScalarType) -> String {
    if ty == ScalarType::BOOL {
        return (bits != 0).to_string();
    }
    let n = if ty.signed && (1..=128).contains(&ty.bits) {
        (((bits << (128 - ty.bits)) as i128) >> (128 - ty.bits)).to_string()
    } else {
        bits.to_string()
    };
    format!("{n}{}", scalar(ty))
}
struct Renderer<'a> {
    bindings: HashMap<u32, &'a SolverBinding>,
    used: BTreeSet<u32>,
}
impl Renderer<'_> {
    fn formula(&mut self, f: &Formula) -> String {
        match f {
            Formula::Literal(b) => b.to_string(),
            // Drop the Boolean-to-bit wrapper used by the solver representation.
            Formula::Compare(C::Eq, Term::Boolean(p), Term::Constant(1, t))
                if *t == ScalarType::BOOL =>
            {
                self.formula(p)
            }
            Formula::Compare(c, a, b) => format!("({} {} {})", self.term(a), cmp(*c), self.term(b)),
            Formula::MathCompare(c, a, b) => {
                format!("({} {} {})", self.integer(a), cmp(*c), self.integer(b))
            }
            Formula::FloatCompare(c, a, b) => {
                format!("({} {} {})", self.float(a), cmp(*c), self.float(b))
            }
            Formula::FloatSame(a, b) => {
                format!("same_float_bits({}, {})", self.float(a), self.float(b))
            }
            Formula::IsNan(a) => format!("is_nan({})", self.float(a)),
            Formula::IsInfinite(a) => format!("is_infinite({})", self.float(a)),
            Formula::Not(p) => format!("!{}", self.formula(p)),
            Formula::All(ps) | Formula::Any(ps) => {
                let all = matches!(f, Formula::All(_));
                if ps.is_empty() {
                    return all.to_string();
                }
                format!(
                    "({})",
                    ps.iter()
                        .map(|p| self.formula(p))
                        .collect::<Vec<_>>()
                        .join(if all { " && " } else { " || " })
                )
            }
        }
    }
    fn term(&mut self, t: &Term) -> String {
        match t {
            Term::Variable(id, _) => {
                self.used.insert(*id);
                self.bindings
                    .get(id)
                    .map(|b| name(&b.subject))
                    .unwrap_or_else(|| format!("v{id}"))
            }
            Term::Constant(n, t) => constant(*n, *t),
            Term::Binary(o, a, b) => format!(
                "bits::<{}>({} {} {})",
                t.scalar_type().map(scalar).unwrap_or_else(|| "?".into()),
                self.term(a),
                op(*o),
                self.term(b)
            ),
            Term::Unary(o, a) => {
                format!("({}{})", if *o == U::Not { "!" } else { "-" }, self.term(a))
            }
            Term::Cast(a, t) => format!("({} as {})", self.term(a), scalar(*t)),
            Term::Boolean(p) => self.formula(p),
            Term::Overflow(o, a, b) => {
                format!("overflow({} {} {})", self.term(a), op(*o), self.term(b))
            }
            Term::FromInteger(a, t) => format!("bits::<{}>({})", scalar(*t), self.integer(a)),
            Term::FloatBits(a) => format!("float_bits({})", self.float(a)),
            Term::FloatToInteger(a, t) => format!("({} as {})", self.float(a), scalar(*t)),
            Term::Select(id, i, t) => format!("array{id}[{}]:{}", self.term(i), scalar(*t)),
        }
    }
    fn integer(&mut self, t: &IntTerm) -> String {
        match t {
            IntTerm::Variable(id) => format!("math_var{id}"),
            IntTerm::Constant(n) => n.clone(),
            IntTerm::FromBits(t) => format!("math({})", self.term(t)),
            IntTerm::Binary(o, a, b) => {
                format!("({} {} {})", self.integer(a), op(*o), self.integer(b))
            }
            IntTerm::Neg(a) => format!("(-{})", self.integer(a)),
            IntTerm::Bitwise(o, a, b, w) => format!(
                "math_bits::<{w}>({} {} {})",
                self.integer(a),
                op(*o),
                self.integer(b)
            ),
        }
    }
    fn float(&mut self, t: &FloatTerm) -> String {
        match t {
            FloatTerm::FromBits(t) => format!("float_from_bits({})", self.term(t)),
            FloatTerm::FromInteger(t, w) => format!("({} as f{w})", self.term(t)),
            FloatTerm::FromMath(t, w) => format!("({} as f{w})", self.integer(t)),
            FloatTerm::Cast(t, w) => format!("({} as f{w})", self.float(t)),
            FloatTerm::Binary(o, a, b) => {
                format!("({} {} {})", self.float(a), op(*o), self.float(b))
            }
            FloatTerm::Neg(a) => format!("(-{})", self.float(a)),
        }
    }
}

impl std::fmt::Display for ExpressionEvidence {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Abstract {
                facts,
                goal,
                result,
            } => {
                writeln!(f, "  kind: abstract")?;
                for fact in facts {
                    writeln!(f, "  {fact}")?;
                }
                write!(
                    f,
                    "  {goal} => {}",
                    result
                        .map(|b| b.to_string())
                        .unwrap_or_else(|| "unknown".into())
                )
            }
            Self::Smt {
                variables,
                assumptions,
                goal,
                negative,
                positive,
            } => {
                writeln!(f, "  kind: smt")?;
                for variable in variables {
                    writeln!(f, "  variable: {variable}")?;
                }
                for assumption in assumptions {
                    writeln!(f, "  assume: {assumption}")?;
                }
                writeln!(f, "  goal: {goal}")?;
                write!(f, "  A && !goal: {negative:?}, A && goal: {positive:?}")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn machine_signedness_and_all_assumptions_survive_projection() {
        let ty = ScalarType {
            bits: 8,
            signed: true,
        };
        let binding = SolverBinding {
            variable: 0,
            subject: Expression::variable("$index"),
            scalar_type: ty,
        };
        let p = Formula::Compare(C::Lt, Term::Variable(0, ty), Term::Constant(255, ty));
        let mut r = Renderer {
            bindings: HashMap::from([(0, &binding)]),
            used: BTreeSet::new(),
        };
        assert_eq!(r.formula(&p), "(index < -1i8)");
        let evidence = Evidence {
            expressions: None,
            abstract_evaluation: None,
            context: vec![],
            smt: Some(SolverEvidence {
                problem: mirsa_solver::Problem {
                    assumptions: vec![p.clone(), p],
                    property: Formula::Literal(false),
                },
                bindings: vec![binding],
                checks: None,
            }),
        };
        let Some(ExpressionEvidence::Smt {
            assumptions,
            variables,
            goal,
            ..
        }) = evidence.expression_view()
        else {
            panic!()
        };
        assert_eq!(assumptions, vec!["(index < -1i8)", "(index < -1i8)"]);
        assert_eq!(variables, vec!["index: i8"]);
        assert_eq!(goal, "false");
    }
}
