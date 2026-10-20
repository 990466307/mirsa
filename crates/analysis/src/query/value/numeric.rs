use super::*;

impl<'tcx> Eval<'_, 'tcx> {
    pub(super) fn normalize(&self, value: Value, ty: ValueType<'tcx>) -> Value {
        let ValueType::Rust(ty) = ty else {
            return value;
        };
        let Some((min, max)) = bounds(self.tcx, ty) else {
            return value;
        };
        let Value::Integer(a, b) = value else {
            return value;
        };
        if a >= min && b <= max {
            return Value::Integer(a, b);
        }
        if a == b {
            let width = &max - &min + 1;
            let v = ((&a - &min) % &width + &width) % &width + &min;
            return Value::constant(v);
        }
        Value::Integer(min, max)
    }

    pub(super) fn cast(&self, value: Value, target: Ty<'tcx>) -> Value {
        if target.is_floating_point() {
            let kind = float_kind_for_ty(target).unwrap();
            return match value {
                Value::Float(f) => Value::Float(FloatInterval::new(
                    kind.round(f.low),
                    kind.round(f.high),
                    f.may_nan,
                )),
                Value::Integer(a, b) if a == b && a.bits() <= 53 => a
                    .to_f64()
                    .map(|f| Value::Float(FloatInterval::singleton(kind.round(f))))
                    .unwrap_or(Value::Unknown),
                _ => Value::Float(FloatInterval::top()),
            };
        }
        match value {
            Value::Bool(v) => v
                .map(|v| Value::constant(u8::from(v)))
                .unwrap_or(Value::Integer(0.into(), 1.into())),
            Value::Integer(..) => self.normalize(value, ValueType::Rust(target)),
            Value::Float(f) if f.is_numeric_singleton() => {
                let Some((lo, hi)) = bounds(self.tcx, target) else {
                    return Value::Unknown;
                };
                if f.low == f64::INFINITY {
                    return Value::constant(hi);
                }
                if f.low == f64::NEG_INFINITY {
                    return Value::constant(lo);
                }
                use num_traits::FromPrimitive;
                BigInt::from_f64(f.low.trunc())
                    .map(|n| Value::constant(n.max(lo).min(hi)))
                    .unwrap_or(Value::Unknown)
            }
            Value::Float(f) if !f.has_numeric_values() && f.may_nan => Value::constant(0),
            Value::Float(_) => bounds(self.tcx, target)
                .map(|(a, b)| Value::Integer(a, b))
                .unwrap_or(Value::Unknown),
            _ => Value::Unknown,
        }
    }

    pub(super) fn binary(&self, op: BinaryOp, a: &Expr<'tcx>, b: &Expr<'tcx>) -> Value {
        let (a_value, b_value) = (self.eval(a), self.eval(b));
        if let (Value::Bool(a), Value::Bool(b)) = (&a_value, &b_value) {
            return Value::Bool(match op {
                BinaryOp::And => {
                    if *a == Some(false) || *b == Some(false) {
                        Some(false)
                    } else {
                        a.zip(*b).map(|(a, b)| a & b)
                    }
                }
                BinaryOp::Or => {
                    if *a == Some(true) || *b == Some(true) {
                        Some(true)
                    } else {
                        a.zip(*b).map(|(a, b)| a | b)
                    }
                }
                BinaryOp::Xor => a.zip(*b).map(|(a, b)| a ^ b),
                _ => None,
            });
        }
        if let (Value::Float(x), Value::Float(y)) = (&a_value, &b_value) {
            let ValueType::Rust(t) = ty(self.tcx, self.body, a) else {
                return Value::Unknown;
            };
            let Some(kind) = float_kind_for_ty(t) else {
                return Value::Unknown;
            };
            return Value::Float(match op {
                BinaryOp::Add => float_interval::add(kind, x, y),
                BinaryOp::Sub => float_interval::sub(kind, x, y),
                BinaryOp::Mul => float_interval::mul(kind, x, y),
                BinaryOp::Div => float_interval::div(kind, x, y),
                BinaryOp::Rem if x.is_numeric_singleton() && y.is_numeric_singleton() => {
                    FloatInterval::singleton(kind.round(x.low % y.low))
                }
                _ => FloatInterval::top(),
            });
        }
        let (Some((al, ah)), Some((bl, bh))) = (a_value.integer(), b_value.integer()) else {
            return Value::Unknown;
        };
        let result = match op {
            BinaryOp::Add => Value::Integer(al + bl, ah + bh),
            BinaryOp::Sub => Value::Integer(al - bh, ah - bl),
            BinaryOp::Mul => {
                let products = [al * bl, al * bh, ah * bl, ah * bh];
                Value::Integer(
                    products.iter().min().unwrap().clone(),
                    products.iter().max().unwrap().clone(),
                )
            }
            BinaryOp::Div | BinaryOp::Rem => {
                if bl <= &BigInt::zero() && bh >= &BigInt::zero() {
                    return Value::Unknown;
                }
                if let ValueType::Rust(t) = ty(self.tcx, self.body, a) {
                    if let Some((min, _)) = bounds(self.tcx, t) {
                        if min < 0.into()
                            && al <= &min
                            && ah >= &min
                            && bl <= &(-BigInt::one())
                            && bh >= &(-BigInt::one())
                        {
                            return Value::Unknown;
                        }
                    }
                }
                if matches!(op, BinaryOp::Div) {
                    let r = [al / bl, al / bh, ah / bl, ah / bh];
                    Value::Integer(
                        r.iter().min().unwrap().clone(),
                        r.iter().max().unwrap().clone(),
                    )
                } else if al == ah && bl == bh {
                    Value::constant(al % bl)
                } else {
                    return Value::Unknown;
                }
            }
            BinaryOp::And | BinaryOp::Or | BinaryOp::Xor if al == ah && bl == bh => {
                Value::constant(match op {
                    BinaryOp::And => al & bl,
                    BinaryOp::Or => al | bl,
                    _ => al ^ bl,
                })
            }
            BinaryOp::Shl | BinaryOp::Shr if al == ah && bl == bh => {
                let Some(n) = bl.to_u32() else {
                    return Value::Unknown;
                };
                let limit = match ty(self.tcx, self.body, a) {
                    ValueType::Rust(t) => abstract_query::scalar_type(self.tcx, t)
                        .map(|t| t.bits)
                        .unwrap_or(0),
                    ValueType::Math => 4096,
                };
                if n >= limit {
                    return Value::Unknown;
                }
                Value::constant(if op == BinaryOp::Shl {
                    al << n
                } else {
                    al >> n
                })
            }
            _ => return Value::Unknown,
        };
        if let Value::Integer(a, b) = &result {
            if a.bits() > 8192 || b.bits() > 8192 {
                return Value::Unknown;
            }
        }
        self.normalize(result, ty(self.tcx, self.body, a))
    }
}
