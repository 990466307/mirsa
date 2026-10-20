use super::*;
use num_bigint::BigInt;

impl<'tcx> Encoder<'_, 'tcx> {
    pub(super) fn integer_bounds(&mut self, expr: &Expr<'tcx>) -> Result<(BigInt, BigInt)> {
        self.tick()?;
        if let Value::Integer(lo, hi) = self.eval().eval(expr) {
            return Ok((lo, hi));
        }
        let bound = match expr {
            Expr::Unary(op, a) => {
                let (lo, hi) = self.integer_bounds(a)?;
                match op {
                    UnaryOp::Neg => (-hi, -lo),
                    UnaryOp::Not => (!hi, !lo),
                }
            }
            Expr::Binary(op, a, b) => {
                let (al, ah) = self.integer_bounds(a)?;
                let (bl, bh) = self.integer_bounds(b)?;
                match op {
                    BinaryOp::Add => (al + bl, ah + bh),
                    BinaryOp::Sub => (al - bh, ah - bl),
                    BinaryOp::Mul => {
                        let values = [&al * &bl, &al * &bh, &ah * &bl, &ah * &bh];
                        (
                            values.iter().min().unwrap().clone(),
                            values.iter().max().unwrap().clone(),
                        )
                    }
                    BinaryOp::And
                    | BinaryOp::Or
                    | BinaryOp::Xor
                    | BinaryOp::Shl
                    | BinaryOp::Shr => {
                        let mut width = al.bits().max(ah.bits()).max(bl.bits()).max(bh.bits()) + 1;
                        if matches!(op, BinaryOp::Shl | BinaryOp::Shr) {
                            if bl < 0.into() || bh >= 4096.into() {
                                return Err(UnknownReason::Unsupported);
                            }
                            if *op == BinaryOp::Shl {
                                width += bh.to_u64().unwrap();
                            }
                        }
                        if width > 8192 {
                            return Err(UnknownReason::BudgetExceeded);
                        }
                        let max = BigInt::from(1) << width;
                        (-&max, max - 1)
                    }
                    _ => return Err(UnknownReason::Unsupported),
                }
            }
            _ => return Err(UnknownReason::Unsupported),
        };
        if bound.0.bits().max(bound.1.bits()) > 8192 {
            return Err(UnknownReason::BudgetExceeded);
        }
        Ok(bound)
    }
}

impl<'tcx> Encoder<'_, 'tcx> {
    pub(super) fn check_defined(&self, op: BinaryOp, a: &Expr<'tcx>, b: &Expr<'tcx>) -> Result<()> {
        if !matches!(
            op,
            BinaryOp::Div | BinaryOp::Rem | BinaryOp::Shl | BinaryOp::Shr
        ) {
            return Ok(());
        }
        let eval = Eval {
            tcx: self.tcx,
            body: self.body,
            state: self.state,
        };
        let right = eval.eval(b);
        let (lo, hi) = right.integer().ok_or(UnknownReason::Unsupported)?;
        if matches!(op, BinaryOp::Shl | BinaryOp::Shr) {
            let bits = abstract_query::expr_type(self.tcx, self.body, a)
                .ok_or(UnknownReason::Unsupported)?
                .bits;
            if lo < &0.into() || hi >= &bits.into() {
                return Err(UnknownReason::Unsupported);
            }
        } else {
            if lo <= &0.into() && hi >= &0.into() {
                return Err(UnknownReason::Unsupported);
            }
            let scalar = abstract_query::expr_type(self.tcx, self.body, a)
                .ok_or(UnknownReason::Unsupported)?;
            if scalar.signed && lo <= &(-1).into() && hi >= &(-1).into() {
                let left = eval.eval(a);
                let (ll, lh) = left.integer().ok_or(UnknownReason::Unsupported)?;
                let min = -(num_bigint::BigInt::from(1) << (scalar.bits - 1));
                if ll <= &min && lh >= &min {
                    return Err(UnknownReason::Unsupported);
                }
            }
        }
        Ok(())
    }

    pub(super) fn integer(&mut self, expr: &Expr<'tcx>) -> Result<IntTerm> {
        self.tick()?;
        if value::ty(self.tcx, self.body, expr) != ValueType::Math {
            return Ok(IntTerm::FromBits(self.expr(expr, 0)?));
        }
        match expr {
            Expr::Binary(op, a, b) => {
                if matches!(op, BinaryOp::Div | BinaryOp::Rem) {
                    let value = self.eval().eval(b);
                    let (lo, hi) = value.integer().ok_or(UnknownReason::Unsupported)?;
                    if lo <= &0.into() && hi >= &0.into() {
                        return Err(UnknownReason::Unsupported);
                    }
                }
                let bits = if matches!(
                    op,
                    BinaryOp::And | BinaryOp::Or | BinaryOp::Xor | BinaryOp::Shl | BinaryOp::Shr
                ) {
                    let (al, ah) = self.integer_bounds(a)?;
                    let (bl, bh) = self.integer_bounds(b)?;
                    let mut bits = al.bits().max(ah.bits()).max(bl.bits()).max(bh.bits()) + 2;
                    if matches!(op, BinaryOp::Shl | BinaryOp::Shr) {
                        if bl < 0.into() || bh >= 4096.into() {
                            return Err(UnknownReason::Unsupported);
                        }
                        if *op == BinaryOp::Shl {
                            bits += bh.to_u64().ok_or(UnknownReason::Unsupported)?;
                        }
                    }
                    if bits > 8193 {
                        return Err(UnknownReason::BudgetExceeded);
                    }
                    Some(bits as u32)
                } else {
                    None
                };
                let a = Box::new(self.integer(a)?);
                let b = Box::new(self.integer(b)?);
                Ok(if let Some(bits) = bits {
                    IntTerm::Bitwise(*op, a, b, bits)
                } else {
                    IntTerm::Binary(*op, a, b)
                })
            }
            Expr::Unary(UnaryOp::Neg, a) => Ok(IntTerm::Neg(Box::new(self.integer(a)?))),
            Expr::Unary(UnaryOp::Not, a) => Ok(IntTerm::Binary(
                BinaryOp::Sub,
                Box::new(IntTerm::Neg(Box::new(self.integer(a)?))),
                Box::new(IntTerm::Constant("1".into())),
            )),
            Expr::TypedExpr(e) => match &e.kind {
                TypedKind::Integer(n) => Ok(IntTerm::Constant(n.to_string())),
                TypedKind::Builtin(op, args) => self.integer_builtin(expr, *op, args),
                _ => Err(UnknownReason::Unsupported),
            },
            _ => Err(UnknownReason::Unsupported),
        }
    }
}
