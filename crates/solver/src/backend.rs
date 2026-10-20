use crate::{
    BinaryOp, Comparison, Formula, IntTerm, Problem, ScalarType, SolveResult, Term, UnaryOp,
    UnknownReason, Verdict,
};
use std::collections::HashMap;
use std::time::{Duration, Instant};
use z3::{
    Config, Context, Params, SatResult, Solver, Sort,
    ast::{Array, Ast, BV, Bool, Int},
};

mod float;
mod integer;

struct Encoder<'ctx> {
    ctx: &'ctx Context,
    variables: HashMap<u32, (ScalarType, BV<'ctx>)>,
    arrays: HashMap<u32, (ScalarType, ScalarType, Array<'ctx>)>,
}

impl<'ctx> Encoder<'ctx> {
    fn constant(&self, bits: u128, ty: ScalarType) -> BV<'ctx> {
        if ty.bits <= 64 {
            BV::from_u64(self.ctx, bits as u64, ty.bits)
        } else {
            BV::from_u64(self.ctx, (bits >> 64) as u64, ty.bits - 64).concat(&BV::from_u64(
                self.ctx,
                bits as u64,
                64,
            ))
        }
    }

    fn resize(value: BV<'ctx>, bits: u32, signed: bool) -> BV<'ctx> {
        match value.get_size().cmp(&bits) {
            std::cmp::Ordering::Equal => value,
            std::cmp::Ordering::Greater => value.extract(bits - 1, 0),
            std::cmp::Ordering::Less => {
                let extra = bits - value.get_size();
                if signed {
                    value.sign_ext(extra)
                } else {
                    value.zero_ext(extra)
                }
            }
        }
    }

    fn term(&mut self, term: &Term) -> Option<BV<'ctx>> {
        let ty = term.scalar_type()?;
        Some(match term {
            Term::Variable(id, _) => {
                if let Some((existing, value)) = self.variables.get(id) {
                    if *existing != ty {
                        return None;
                    }
                    value.clone()
                } else {
                    let value = BV::new_const(self.ctx, format!("v{id}"), ty.bits);
                    self.variables.insert(*id, (ty, value.clone()));
                    value
                }
            }
            Term::Constant(bits, _) => self.constant(*bits, ty),
            Term::Select(id, index, _) => {
                let index_ty = index.scalar_type()?;
                let index = self.term(index)?;
                let array =
                    if let Some((existing_index, existing_value, array)) = self.arrays.get(id) {
                        if *existing_index != index_ty || *existing_value != ty {
                            return None;
                        }
                        array.clone()
                    } else {
                        let array = Array::new_const(
                            self.ctx,
                            format!("a{id}"),
                            &Sort::bitvector(self.ctx, index_ty.bits),
                            &Sort::bitvector(self.ctx, ty.bits),
                        );
                        self.arrays.insert(*id, (index_ty, ty, array.clone()));
                        array
                    };
                array.select(&index).as_bv()?
            }
            Term::FromInteger(value, _) => self.integer_bits(value, ty.bits)?,
            Term::FloatBits(value) => {
                let value = self.float(value)?;
                value.is_nan().ite(
                    &self.constant(
                        if ty.bits == 32 {
                            0x7fc00000
                        } else {
                            0x7ff8000000000000
                        },
                        ty,
                    ),
                    &value.to_ieee_bv(),
                )
            }
            Term::FloatToInteger(value, _) => self.float_to_integer(value, ty)?,
            Term::Unary(op, value) => {
                let value = self.term(value)?;
                match op {
                    UnaryOp::Neg => value.bvneg(),
                    UnaryOp::Not => value.bvnot(),
                }
            }
            Term::Cast(value, _) => {
                let source = value.scalar_type()?;
                Self::resize(self.term(value)?, ty.bits, source.signed)
            }
            Term::Boolean(formula) => self.formula(formula)?.ite(
                &self.constant(1, ScalarType::BOOL),
                &self.constant(0, ScalarType::BOOL),
            ),
            Term::Binary(op, left, right) => {
                let left = self.term(left)?;
                let mut right = self.term(right)?;
                if matches!(op, BinaryOp::Shl | BinaryOp::Shr) {
                    right = Self::resize(right, ty.bits, false)
                        .bvand(&self.constant(u128::from(ty.bits - 1), ty));
                }
                match op {
                    BinaryOp::Add => left.bvadd(&right),
                    BinaryOp::Sub => left.bvsub(&right),
                    BinaryOp::Mul => left.bvmul(&right),
                    BinaryOp::And => left.bvand(&right),
                    BinaryOp::Or => left.bvor(&right),
                    BinaryOp::Xor => left.bvxor(&right),
                    BinaryOp::Shl => left.bvshl(&right),
                    BinaryOp::Shr if ty.signed => left.bvashr(&right),
                    BinaryOp::Shr => left.bvlshr(&right),
                    BinaryOp::Div if ty.signed => left.bvsdiv(&right),
                    BinaryOp::Div => left.bvudiv(&right),
                    BinaryOp::Rem if ty.signed => left.bvsrem(&right),
                    BinaryOp::Rem => left.bvurem(&right),
                }
            }
            Term::Overflow(op, left, right) => {
                let source = left.scalar_type()?;
                let a = Self::resize(self.term(left)?, source.bits * 2, source.signed);
                let b = Self::resize(self.term(right)?, source.bits * 2, source.signed);
                let wide = match op {
                    BinaryOp::Add => a.bvadd(&b),
                    BinaryOp::Sub => a.bvsub(&b),
                    BinaryOp::Mul => a.bvmul(&b),
                    _ => return None,
                };
                let narrow = wide.extract(source.bits - 1, 0);
                wide._eq(&Self::resize(narrow, source.bits * 2, source.signed))
                    .not()
                    .ite(
                        &self.constant(1, ScalarType::BOOL),
                        &self.constant(0, ScalarType::BOOL),
                    )
            }
        })
    }

    fn integer(&mut self, term: &IntTerm) -> Option<Int<'ctx>> {
        Some(match term {
            IntTerm::Variable(id) => Int::new_const(self.ctx, format!("i{id}")),
            IntTerm::Constant(n) => Int::from_str(self.ctx, n)?,
            IntTerm::FromBits(t) => Int::from_bv(&self.term(t)?, t.scalar_type()?.signed),
            IntTerm::Neg(n) => self.integer(n)?.unary_minus(),
            IntTerm::Binary(op, a, b) => {
                let a = self.integer(a)?;
                let b = self.integer(b)?;
                match op {
                    BinaryOp::Add => Int::add(self.ctx, &[&a, &b]),
                    BinaryOp::Sub => Int::sub(self.ctx, &[&a, &b]),
                    BinaryOp::Mul => Int::mul(self.ctx, &[&a, &b]),
                    BinaryOp::Div | BinaryOp::Rem => {
                        let zero = Int::from_i64(self.ctx, 0);
                        let abs_a = a.ge(&zero).ite(&a, &a.unary_minus());
                        let abs_b = b.ge(&zero).ite(&b, &b.unary_minus());
                        let q = abs_a.div(&abs_b);
                        let q = a.ge(&zero)._eq(&b.ge(&zero)).ite(&q, &q.unary_minus());
                        if *op == BinaryOp::Div {
                            q
                        } else {
                            Int::sub(self.ctx, &[&a, &Int::mul(self.ctx, &[&b, &q])])
                        }
                    }
                    _ => return None,
                }
            }
            IntTerm::Bitwise(_, _, _, bits) => Int::from_bv(&self.integer_bits(term, *bits)?, true),
        })
    }

    fn integer_bits(&mut self, term: &IntTerm, bits: u32) -> Option<BV<'ctx>> {
        Some(match term {
            IntTerm::FromBits(value) => {
                Self::resize(self.term(value)?, bits, value.scalar_type()?.signed)
            }
            IntTerm::Neg(value) => self.integer_bits(value, bits)?.bvneg(),
            IntTerm::Binary(op @ (BinaryOp::Add | BinaryOp::Sub | BinaryOp::Mul), a, b) => {
                let a = self.integer_bits(a, bits)?;
                let b = self.integer_bits(b, bits)?;
                match op {
                    BinaryOp::Add => a.bvadd(&b),
                    BinaryOp::Sub => a.bvsub(&b),
                    _ => a.bvmul(&b),
                }
            }
            IntTerm::Bitwise(op, a, b, width) => {
                if *width == 0 || *width > 8193 {
                    return None;
                }
                let a = self.integer_bits(a, *width)?;
                let b = self.integer_bits(b, *width)?;
                let value = match op {
                    BinaryOp::And => a.bvand(&b),
                    BinaryOp::Or => a.bvor(&b),
                    BinaryOp::Xor => a.bvxor(&b),
                    BinaryOp::Shl => a.bvshl(&b),
                    BinaryOp::Shr => a.bvashr(&b),
                    _ => return None,
                };
                Self::resize(value, bits, true)
            }
            _ => BV::from_int(&self.integer(term)?, bits),
        })
    }

    fn formula(&mut self, formula: &Formula) -> Option<Bool<'ctx>> {
        Some(match formula {
            Formula::FloatCompare(op, a, b) => self.float_compare(*op, a, b)?,
            Formula::FloatSame(a, b) => {
                if a.bits()? != b.bits()? {
                    return None;
                }
                self.float(a)?._eq(&self.float(b)?)
            }
            Formula::IsNan(value) => self.float(value)?.is_nan(),
            Formula::IsInfinite(value) => self.float(value)?.is_infinite(),
            Formula::MathCompare(op, a, b) => {
                let a = self.integer(a)?;
                let b = self.integer(b)?;
                match op {
                    Comparison::Eq => a._eq(&b),
                    Comparison::Ne => a._eq(&b).not(),
                    Comparison::Lt => a.lt(&b),
                    Comparison::Le => a.le(&b),
                    Comparison::Gt => a.gt(&b),
                    Comparison::Ge => a.ge(&b),
                }
            }
            Formula::Literal(value) => Bool::from_bool(self.ctx, *value),
            Formula::Not(value) => self.formula(value)?.not(),
            Formula::All(values) | Formula::Any(values) => {
                let values = values
                    .iter()
                    .map(|v| self.formula(v))
                    .collect::<Option<Vec<_>>>()?;
                let refs: Vec<_> = values.iter().collect();
                if matches!(formula, Formula::All(_)) {
                    Bool::and(self.ctx, &refs)
                } else {
                    Bool::or(self.ctx, &refs)
                }
            }
            Formula::Compare(op, a, b) => {
                let ty = a.scalar_type()?;
                if ty != b.scalar_type()? {
                    return None;
                }
                let a = self.term(a)?;
                let b = self.term(b)?;
                match op {
                    Comparison::Eq => a._eq(&b),
                    Comparison::Ne => a._eq(&b).not(),
                    Comparison::Lt if ty.signed => a.bvslt(&b),
                    Comparison::Le if ty.signed => a.bvsle(&b),
                    Comparison::Gt if ty.signed => a.bvsgt(&b),
                    Comparison::Ge if ty.signed => a.bvsge(&b),
                    Comparison::Lt => a.bvult(&b),
                    Comparison::Le => a.bvule(&b),
                    Comparison::Gt => a.bvugt(&b),
                    Comparison::Ge => a.bvuge(&b),
                }
            }
        })
    }
}

pub(super) fn solve(problem: &Problem, timeout: Duration) -> SolveResult {
    let started = Instant::now();
    let ctx = Context::new(&Config::new());
    let solver = Solver::new(&ctx);
    let mut encoder = Encoder {
        ctx: &ctx,
        variables: HashMap::new(),
        arrays: HashMap::new(),
    };
    let encoded = integer::encode(&ctx, problem).or_else(|| {
        let assumptions = problem
            .assumptions
            .iter()
            .map(|f| encoder.formula(f))
            .collect::<Option<Vec<_>>>()?;
        Some((assumptions, encoder.formula(&problem.property)?))
    });
    let Some((assumptions, property)) = encoded else {
        return SolveResult::unknown(UnknownReason::Unsupported);
    };
    let mut seen = std::collections::HashSet::new();
    for assumption in assumptions {
        let assumption = assumption.simplify();
        if assumption.as_bool() != Some(true) && seen.insert(assumption.clone()) {
            solver.assert(&assumption);
        }
    }
    let check = || {
        let remaining = timeout.saturating_sub(started.elapsed());
        if remaining.is_zero() {
            return SatResult::Unknown;
        }
        let mut params = Params::new(&ctx);
        params.set_u32(
            "timeout",
            remaining.as_millis().clamp(1, u32::MAX as u128) as u32,
        );
        solver.set_params(&params);
        solver.check()
    };
    let unknown = || {
        SolveResult::unknown(if started.elapsed() >= timeout {
            UnknownReason::BudgetExceeded
        } else {
            UnknownReason::SolverUnknown
        })
    };
    solver.push();
    solver.assert(&property.not());
    let negative = check();
    let candidate = match negative {
        SatResult::Unsat => None,
        SatResult::Unknown => return unknown(),
        SatResult::Sat => solver.get_model().map(|model| model.to_string()),
    };
    solver.pop(1);
    solver.assert(&property);
    match (negative, check()) {
        (SatResult::Unsat, SatResult::Sat) => SolveResult::known(Verdict::Proven),
        (SatResult::Sat, SatResult::Unsat) => SolveResult::known(Verdict::Refuted),
        (SatResult::Unsat, SatResult::Unsat) => SolveResult::known(Verdict::Unreachable),
        (negative, positive) => {
            let mut out = if negative == SatResult::Sat && positive == SatResult::Sat {
                SolveResult::unknown(UnknownReason::InsufficientInformation)
            } else {
                unknown()
            };
            out.candidate = candidate;
            out
        }
    }
}
