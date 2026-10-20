use super::*;

pub(super) fn encode<'ctx>(
    ctx: &'ctx Context,
    problem: &Problem,
) -> Option<(Vec<Bool<'ctx>>, Bool<'ctx>)> {
    fn has_math(f: &Formula) -> bool {
        match f {
            Formula::MathCompare(..) => true,
            Formula::All(values) | Formula::Any(values) => values.iter().any(has_math),
            Formula::Not(value) => has_math(value),
            Formula::Compare(_, a, b) => term_has_math(a) || term_has_math(b),
            _ => false,
        }
    }
    fn term_has_math(t: &Term) -> bool {
        match t {
            Term::FromInteger(..) => true,
            Term::Boolean(f) => has_math(f),
            Term::Binary(_, a, b) | Term::Overflow(_, a, b) => term_has_math(a) || term_has_math(b),
            Term::Unary(_, t) | Term::Cast(t, _) | Term::Select(_, t, _) => term_has_math(t),
            _ => false,
        }
    }
    if !problem
        .assumptions
        .iter()
        .chain(std::iter::once(&problem.property))
        .any(has_math)
    {
        return None;
    }
    let mut encoder = IntegerEncoder {
        ctx,
        variables: HashMap::new(),
        bounds: Vec::new(),
    };
    let mut assumptions = problem
        .assumptions
        .iter()
        .map(|f| encoder.formula(f))
        .collect::<Option<Vec<_>>>()?;
    let property = encoder.formula(&problem.property)?;
    assumptions.extend(encoder.bounds);
    Some((assumptions, property))
}

struct IntegerEncoder<'ctx> {
    ctx: &'ctx Context,
    variables: HashMap<u32, (ScalarType, Int<'ctx>)>,
    bounds: Vec<Bool<'ctx>>,
}

impl<'ctx> IntegerEncoder<'ctx> {
    fn number(&self, n: i64) -> Int<'ctx> {
        Int::from_i64(self.ctx, n)
    }

    fn power(&self, bits: u32) -> Option<Int<'ctx>> {
        let value = if bits == 128 {
            "340282366920938463463374607431768211456".to_owned()
        } else if bits < 128 {
            (1_u128 << bits).to_string()
        } else {
            return None;
        };
        Int::from_str(self.ctx, &value)
    }

    fn limits(&self, ty: ScalarType) -> Option<(Int<'ctx>, Int<'ctx>)> {
        let (low, end) = if ty.signed {
            let half = self.power(ty.bits.checked_sub(1)?)?;
            (half.unary_minus(), half)
        } else {
            (self.number(0), self.power(ty.bits)?)
        };
        Some((low, Int::sub(self.ctx, &[&end, &self.number(1)])))
    }

    fn wrap(&self, value: Int<'ctx>, ty: ScalarType) -> Option<Int<'ctx>> {
        let (low, _) = self.limits(ty)?;
        let shifted = Int::sub(self.ctx, &[&value, &low]);
        Some(Int::add(self.ctx, &[&shifted.modulo(&self.power(ty.bits)?), &low]).simplify())
    }

    fn arithmetic(&self, op: BinaryOp, a: &Int<'ctx>, b: &Int<'ctx>) -> Option<Int<'ctx>> {
        Some(match op {
            BinaryOp::Add => Int::add(self.ctx, &[a, b]),
            BinaryOp::Sub => Int::sub(self.ctx, &[a, b]),
            BinaryOp::Mul => Int::mul(self.ctx, &[a, b]),
            BinaryOp::Div | BinaryOp::Rem => {
                let zero = self.number(0);
                let abs_a = a.ge(&zero).ite(a, &a.unary_minus());
                let abs_b = b.ge(&zero).ite(b, &b.unary_minus());
                let quotient = abs_a.div(&abs_b);
                let quotient = a
                    .ge(&zero)
                    ._eq(&b.ge(&zero))
                    .ite(&quotient, &quotient.unary_minus());
                if op == BinaryOp::Div {
                    quotient
                } else {
                    Int::sub(self.ctx, &[a, &Int::mul(self.ctx, &[b, &quotient])])
                }
            }
            _ => return None,
        })
    }

    fn term(&mut self, term: &Term) -> Option<Int<'ctx>> {
        let ty = term.scalar_type()?;
        Some(match term {
            Term::Variable(id, _) => {
                if let Some((existing, value)) = self.variables.get(id) {
                    if *existing != ty {
                        return None;
                    }
                    value.clone()
                } else {
                    let value = Int::new_const(self.ctx, format!("v{id}"));
                    let (low, high) = self.limits(ty)?;
                    self.bounds.extend([value.ge(&low), value.le(&high)]);
                    self.variables.insert(*id, (ty, value.clone()));
                    value
                }
            }
            Term::Constant(bits, _) => {
                self.wrap(Int::from_str(self.ctx, &bits.to_string())?, ty)?
            }
            Term::Binary(op @ (BinaryOp::Shl | BinaryOp::Shr), a, b) => {
                let Term::Constant(bits, rhs) = b.as_ref() else {
                    return None;
                };
                let bits = if rhs.bits < 128 {
                    bits & ((1_u128 << rhs.bits) - 1)
                } else {
                    *bits
                };
                let scale = self.power((bits & u128::from(ty.bits.checked_sub(1)?)) as u32)?;
                let a = self.term(a)?;
                if *op == BinaryOp::Shl {
                    self.wrap(Int::mul(self.ctx, &[&a, &scale]), ty)?
                } else {
                    a.div(&scale)
                }
            }
            Term::Binary(op, a, b) => {
                let a = self.term(a)?;
                let b = self.term(b)?;
                let arithmetic = match op {
                    BinaryOp::Div if !ty.signed => a.div(&b),
                    BinaryOp::Rem if !ty.signed => a.modulo(&b),
                    _ => self.arithmetic(*op, &a, &b)?,
                };
                let result = self.wrap(arithmetic, ty)?;
                if matches!(op, BinaryOp::Div | BinaryOp::Rem) {
                    let on_zero = if *op == BinaryOp::Rem {
                        a.clone()
                    } else if ty.signed {
                        a.lt(&self.number(0)).ite(&self.number(1), &self.number(-1))
                    } else {
                        self.limits(ty)?.1
                    };
                    b._eq(&self.number(0))
                        .ite(&self.wrap(on_zero, ty)?, &result)
                } else {
                    result
                }
            }
            Term::Unary(op, a) => {
                let a = self.term(a)?.unary_minus();
                self.wrap(
                    match op {
                        UnaryOp::Neg => a,
                        UnaryOp::Not => Int::sub(self.ctx, &[&a, &self.number(1)]),
                    },
                    ty,
                )?
            }
            Term::Cast(a, _) => {
                let a = self.term(a)?;
                self.wrap(a, ty)?
            }
            Term::Boolean(f) => self.formula(f)?.ite(&self.number(1), &self.number(0)),
            Term::Overflow(op, a, b) => {
                let source = a.scalar_type()?;
                let a = self.term(a)?;
                let b = self.term(b)?;
                let value = self.arithmetic(*op, &a, &b)?;
                let (low, high) = self.limits(source)?;
                Bool::or(self.ctx, &[&value.lt(&low), &value.gt(&high)])
                    .ite(&self.number(1), &self.number(0))
            }
            Term::FromInteger(value, _) => {
                let value = self.integer(value)?;
                self.wrap(value, ty)?
            }
            _ => return None,
        })
    }

    fn integer(&mut self, term: &IntTerm) -> Option<Int<'ctx>> {
        Some(match term {
            IntTerm::Variable(id) => Int::new_const(self.ctx, format!("i{id}")),
            IntTerm::Constant(n) => Int::from_str(self.ctx, n)?,
            IntTerm::FromBits(t) => self.term(t)?,
            IntTerm::Neg(t) => self.integer(t)?.unary_minus(),
            IntTerm::Binary(op, a, b) => {
                let a = self.integer(a)?;
                let b = self.integer(b)?;
                self.arithmetic(*op, &a, &b)?
            }
            _ => return None,
        })
    }

    fn compare(&self, op: Comparison, a: Int<'ctx>, b: Int<'ctx>) -> Bool<'ctx> {
        match op {
            Comparison::Eq => a._eq(&b),
            Comparison::Ne => a._eq(&b).not(),
            Comparison::Lt => a.lt(&b),
            Comparison::Le => a.le(&b),
            Comparison::Gt => a.gt(&b),
            Comparison::Ge => a.ge(&b),
        }
    }

    fn formula(&mut self, formula: &Formula) -> Option<Bool<'ctx>> {
        Some(match formula {
            Formula::Literal(value) => Bool::from_bool(self.ctx, *value),
            Formula::Not(value) => self.formula(value)?.not(),
            Formula::All(values) | Formula::Any(values) => {
                let values = values
                    .iter()
                    .map(|f| self.formula(f))
                    .collect::<Option<Vec<_>>>()?;
                let refs: Vec<_> = values.iter().collect();
                if matches!(formula, Formula::All(_)) {
                    Bool::and(self.ctx, &refs)
                } else {
                    Bool::or(self.ctx, &refs)
                }
            }
            Formula::Compare(op, a, b) => {
                if a.scalar_type()? != b.scalar_type()? {
                    return None;
                }
                let a = self.term(a)?;
                let b = self.term(b)?;
                self.compare(*op, a, b)
            }
            Formula::MathCompare(op, a, b) => {
                let a = self.integer(a)?;
                let b = self.integer(b)?;
                self.compare(*op, a, b)
            }
            _ => return None,
        })
    }
}
