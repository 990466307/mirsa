use super::*;
use mirsa_relations::{
    symbolic::{SymbolicExpr, SymbolicFact},
    value_expr::{CheckedComponent, ValueExpr},
};
use rustc_middle::mir::{BinOp, UnOp};

impl<'tcx> Encoder<'_, 'tcx> {
    pub(super) fn symbolic(&mut self, expr: &SymbolicExpr<'tcx>, depth: usize) -> Result<Term> {
        self.tick()?;
        Ok(match expr {
            SymbolicExpr::Value(value) => match value {
                ValueExpr::Use(operand) => self.operand(operand, depth)?,
                ValueExpr::Binary { op, left, right }
                | ValueExpr::CheckedBinary {
                    op,
                    left,
                    right,
                    component: CheckedComponent::Value,
                } => self.expr(
                    &Expr::Binary(
                        binary(*op)?,
                        Box::new(Expr::Value(left.clone())),
                        Box::new(Expr::Value(right.clone())),
                    ),
                    depth,
                )?,
                ValueExpr::CheckedBinary {
                    op,
                    left,
                    right,
                    component: CheckedComponent::Overflow,
                } => Term::Overflow(
                    binary(*op)?,
                    Box::new(self.operand(left, depth)?),
                    Box::new(self.operand(right, depth)?),
                ),
                ValueExpr::Unary { op, operand } => self.expr(
                    &Expr::Unary(
                        match op {
                            UnOp::Neg => UnaryOp::Neg,
                            UnOp::Not => UnaryOp::Not,
                            _ => return Err(UnknownReason::Unsupported),
                        },
                        Box::new(Expr::Value(operand.clone())),
                    ),
                    depth,
                )?,
                ValueExpr::Cast {
                    operand,
                    target,
                    kind,
                } => {
                    if !matches!(
                        kind,
                        rustc_middle::mir::CastKind::IntToInt
                            | rustc_middle::mir::CastKind::IntToFloat
                            | rustc_middle::mir::CastKind::FloatToInt
                            | rustc_middle::mir::CastKind::FloatToFloat
                    ) {
                        return Err(UnknownReason::Unsupported);
                    }
                    self.expr(
                        &Expr::Cast(Box::new(Expr::Value(operand.clone())), *target),
                        depth,
                    )?
                }
            },
            SymbolicExpr::Cmp { op, left, right } => {
                Term::Boolean(Box::new(self.property(&Predicate::Compare(
                    comparison(*op)?,
                    Expr::Value(left.clone()),
                    Expr::Value(right.clone()),
                ))?))
            }
            SymbolicExpr::Length { place } => self.variable(*place, true, depth)?,
            SymbolicExpr::Address { pointer } => self.address(&Expr::Value(pointer.clone()))?,
            _ => return Err(UnknownReason::Unsupported),
        })
    }

    pub(super) fn collect(&mut self) -> Result<()> {
        let mut cursor = 0;
        while cursor < self.pending.len() {
            self.tick()?;
            let (place, depth) = self.pending[cursor];
            cursor += 1;
            for fact in self
                .state
                .symbolic
                .predicates_for_place(place, self.config.max_values)
            {
                if !self.predicates.contains(&fact) {
                    self.predicates.push(fact.clone());
                    let predicate = super::super::guidance::bind_fact(self.tcx, self.body, &fact)
                        .map_err(|_| UnknownReason::Unsupported)?;
                    let formula = self.property(&predicate)?;
                    self.assumptions.push(formula);
                }
            }
            if let Ok(term) = self.variable(place, false, depth) {
                self.element_fact(place, &term, depth)?;
                let is_float = place
                    .ty(&self.body.local_decls, self.tcx)
                    .ty
                    .is_floating_point();
                if is_float {
                    self.float_facts(place, &term);
                }
                if !is_float {
                    if let Some(range) = self
                        .state
                        .interval
                        .tracked_interval_resolved(&self.state.symbolic, &place)
                    {
                        self.interval(&term, range);
                    }
                    if let Some(bits) = self
                        .state
                        .interval
                        .integer_bits_resolved(&self.state.symbolic, place)
                    {
                        self.assumptions.push(Formula::Compare(
                            Comparison::Eq,
                            term.clone(),
                            Term::Constant(bits.to_bits_unchecked(), term.scalar_type().unwrap()),
                        ));
                    }
                }
                if depth < self.config.max_depth {
                    if let Some(expr) = self.state.symbolic.expr_for_place(place) {
                        if let Ok(value) = self.symbolic(expr, depth + 1) {
                            if value.scalar_type() == term.scalar_type() {
                                self.assumptions.push(if is_float {
                                    Formula::FloatSame(
                                        mirsa_solver::FloatTerm::FromBits(term.clone()),
                                        mirsa_solver::FloatTerm::FromBits(value),
                                    )
                                } else {
                                    Formula::Compare(Comparison::Eq, term.clone(), value)
                                });
                            }
                        }
                    }
                }
                for fact in self
                    .state
                    .symbolic
                    .facts_for_place(place, self.config.max_values)
                {
                    if is_float {
                        continue;
                    }
                    let (op, expr, value) = match fact {
                        SymbolicFact::EqConst { expr, value } => (Comparison::Eq, expr, value),
                        SymbolicFact::NeConst { expr, value } => (Comparison::Ne, expr, value),
                    };
                    if let Ok(value_term) = self.operand(&expr, depth) {
                        let ty = value_term.scalar_type().unwrap();
                        self.assumptions.push(Formula::Compare(
                            op,
                            value_term,
                            Term::Constant(value, ty),
                        ));
                    }
                }
                for equivalent in self
                    .state
                    .symbolic
                    .equivalent_places(place, self.config.max_values)
                {
                    if let Ok(other) = self.variable(equivalent, false, depth + 1) {
                        if term.scalar_type() == other.scalar_type() {
                            self.assumptions.push(if is_float {
                                Formula::FloatSame(
                                    mirsa_solver::FloatTerm::FromBits(term.clone()),
                                    mirsa_solver::FloatTerm::FromBits(other),
                                )
                            } else {
                                Formula::Compare(Comparison::Eq, term.clone(), other)
                            });
                        }
                    }
                }
            }
            if let Some(range) = self
                .state
                .interval
                .tracked_len_resolved(&self.state.symbolic, place)
            {
                if let Ok(term) = self.variable(place, true, depth) {
                    self.interval(&term, range);
                }
            }
            if depth < self.config.max_depth {
                for related in self
                    .state
                    .symbolic
                    .related_places(place, self.config.max_values)
                {
                    self.queue(related, depth + 1);
                }
            }
            if self.assumptions.len() + self.nodes > self.config.max_nodes {
                return Err(UnknownReason::BudgetExceeded);
            }
        }
        Ok(())
    }
}

fn binary(op: BinOp) -> Result<BinaryOp> {
    Ok(match op {
        BinOp::Add | BinOp::AddWithOverflow => BinaryOp::Add,
        BinOp::Sub | BinOp::SubWithOverflow => BinaryOp::Sub,
        BinOp::Mul | BinOp::MulWithOverflow => BinaryOp::Mul,
        BinOp::BitAnd => BinaryOp::And,
        BinOp::BitOr => BinaryOp::Or,
        BinOp::BitXor => BinaryOp::Xor,
        BinOp::Shl => BinaryOp::Shl,
        BinOp::Shr => BinaryOp::Shr,
        BinOp::Div => BinaryOp::Div,
        BinOp::Rem => BinaryOp::Rem,
        _ => return Err(UnknownReason::Unsupported),
    })
}

fn comparison(op: BinOp) -> Result<Comparison> {
    Ok(match op {
        BinOp::Eq => Comparison::Eq,
        BinOp::Ne => Comparison::Ne,
        BinOp::Lt => Comparison::Lt,
        BinOp::Le => Comparison::Le,
        BinOp::Gt => Comparison::Gt,
        BinOp::Ge => Comparison::Ge,
        _ => return Err(UnknownReason::Unsupported),
    })
}
