use super::*;
use mirsa_solver::FloatTerm;
use rustc_middle::ty::Ty;

impl<'tcx> Encoder<'_, 'tcx> {
    pub(super) fn storage_type(&self, ty: Ty<'tcx>) -> Option<ScalarType> {
        if ty == self.tcx.types.f32 {
            Some(ScalarType {
                bits: 32,
                signed: false,
            })
        } else if ty == self.tcx.types.f64 {
            Some(ScalarType {
                bits: 64,
                signed: false,
            })
        } else {
            abstract_query::scalar_type(self.tcx, ty)
        }
    }

    pub(super) fn float_bits(&self, expr: &Expr<'tcx>) -> Option<u32> {
        match value::ty(self.tcx, self.body, expr) {
            ValueType::Rust(t) if t == self.tcx.types.f32 => Some(32),
            ValueType::Rust(t) if t == self.tcx.types.f64 => Some(64),
            _ => None,
        }
    }

    fn float_constant(value: f64, bits: u32) -> FloatTerm {
        FloatTerm::FromBits(Term::Constant(
            if bits == 32 {
                (value as f32).to_bits() as u128
            } else {
                value.to_bits() as u128
            },
            ScalarType {
                bits,
                signed: false,
            },
        ))
    }

    pub(super) fn float(&mut self, expr: &Expr<'tcx>, depth: usize) -> Result<FloatTerm> {
        self.tick()?;
        let bits = self.float_bits(expr).ok_or(UnknownReason::Unsupported)?;
        Ok(match expr {
            Expr::Value(v) => FloatTerm::FromBits(self.operand(v, depth)?),
            Expr::Constant { bits: value, .. } => FloatTerm::FromBits(Term::Constant(
                *value,
                ScalarType {
                    bits,
                    signed: false,
                },
            )),
            Expr::Binary(op, a, b) => FloatTerm::Binary(
                *op,
                Box::new(self.float(a, depth)?),
                Box::new(self.float(b, depth)?),
            ),
            Expr::Unary(UnaryOp::Neg, a) => FloatTerm::Neg(Box::new(self.float(a, depth)?)),
            Expr::Cast(a, _) if self.float_bits(a).is_some() => {
                FloatTerm::Cast(Box::new(self.float(a, depth)?), bits)
            }
            Expr::Cast(a, _) if value::ty(self.tcx, self.body, a) == ValueType::Math => {
                FloatTerm::FromMath(self.integer(a)?, bits)
            }
            Expr::Cast(a, _) => FloatTerm::FromInteger(self.expr(a, depth)?, bits),
            Expr::TypedExpr(t) => match &t.kind {
                TypedKind::Float(value) => Self::float_constant(*value, bits),
                TypedKind::Project { .. } => FloatTerm::FromBits(self.projection(expr, depth)?),
                _ => return Err(UnknownReason::Unsupported),
            },
            _ => return Err(UnknownReason::Unsupported),
        })
    }

    pub(super) fn float_facts(&mut self, place: Place<'tcx>, term: &Term) {
        let Some(range) = self
            .state
            .interval
            .tracked_float_interval_resolved(&self.state.symbolic, &place)
        else {
            return;
        };
        if range.is_bottom() {
            return;
        }
        let value = mirsa_solver::FloatTerm::FromBits(term.clone());
        let bits = term.scalar_type().unwrap().bits;
        let mut alternatives = Vec::new();
        if range.may_nan {
            alternatives.push(Formula::IsNan(value.clone()));
        }
        if range.has_numeric_values() {
            alternatives.push(Formula::All(vec![
                Formula::FloatCompare(
                    Comparison::Ge,
                    value.clone(),
                    Self::float_constant(range.low, bits),
                ),
                Formula::FloatCompare(
                    Comparison::Le,
                    value,
                    Self::float_constant(range.high, bits),
                ),
            ]));
        }
        self.assumptions.push(Formula::Any(alternatives));
    }
}
