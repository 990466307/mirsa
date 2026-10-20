use super::*;

impl<'tcx> Encoder<'_, 'tcx> {
    pub(in crate::query::encode) fn address_relations(
        &mut self,
        expr: &Expr<'tcx>,
        target: &Term,
    ) -> Result<()> {
        use mirsa_relations::symbolic::SymbolicExpr;
        let Term::Variable(id, _) = *target else {
            return Ok(());
        };
        if !self.related_addresses.insert(id) {
            return Ok(());
        }
        let Some(place) = self.eval().place(expr) else {
            return Ok(());
        };
        let Some(SymbolicExpr::Pointer { base, offset }) =
            self.state.symbolic.expr_for_place(place).cloned()
        else {
            return Ok(());
        };
        let source = self.address(&Expr::Value(base))?;
        let address = if let Some((kind, count)) = offset {
            let delta = IntTerm::Binary(
                BinaryOp::Mul,
                Box::new(IntTerm::FromBits(self.operand(&count, 0)?)),
                Box::new(integer(kind.bytes as i128)),
            );
            // Wrapping changes the address modulo pointer width even when the
            // offset cannot be shown to remain within the allocation.
            Term::FromInteger(
                Box::new(IntTerm::Binary(
                    if kind.subtract {
                        BinaryOp::Sub
                    } else {
                        BinaryOp::Add
                    },
                    Box::new(IntTerm::FromBits(source)),
                    Box::new(delta),
                )),
                self.address_type(),
            )
        } else {
            source
        };
        self.assumptions.push(eq(target, &address));
        Ok(())
    }

    pub(super) fn pointer_relations(&mut self, expr: &Expr<'tcx>, pointer: &Pointer) -> Result<()> {
        use mirsa_relations::symbolic::SymbolicExpr;
        if let Some(place) = self.eval().place(expr) {
            if let Some(SymbolicExpr::PointerFromAddress { address }) =
                self.state.symbolic.expr_for_place(place).cloned()
            {
                let address = self.operand(&address, 0)?;
                self.assumptions.push(eq(
                    &pointer.address,
                    &Term::Cast(Box::new(address), self.address_type()),
                ));
            }
            if let Some(SymbolicExpr::Pointer { base, offset }) =
                self.state.symbolic.expr_for_place(place).cloned()
            {
                let source = self.pointer(&Expr::Value(base))?;
                let mut computed = source.offset.clone();
                let mut guard = yes(&source.has_object);
                if let Some((kind, count)) = offset {
                    let delta = IntTerm::Binary(
                        BinaryOp::Mul,
                        Box::new(IntTerm::FromBits(self.operand(&count, 0)?)),
                        Box::new(integer(kind.bytes as i128)),
                    );
                    computed = IntTerm::Binary(
                        if kind.subtract {
                            BinaryOp::Sub
                        } else {
                            BinaryOp::Add
                        },
                        Box::new(computed),
                        Box::new(delta),
                    );
                    if kind.wrapping {
                        guard = Formula::All(vec![
                            guard,
                            Formula::MathCompare(Comparison::Ge, computed.clone(), integer(0)),
                            Formula::MathCompare(
                                Comparison::Le,
                                computed.clone(),
                                source.object.extent.clone(),
                            ),
                        ]);
                    }
                } else {
                    self.assumptions.push(eq(&pointer.address, &source.address));
                }
                self.assumptions.push(implies(
                    guard,
                    Formula::All(vec![
                        yes(&pointer.has_object),
                        ieq(&pointer.object.id, &source.object.id),
                        ieq(&pointer.offset, &computed),
                    ]),
                ));
            }
        }
        if let Some(ty) = self.reference_type(expr) {
            if let TyKind::Ref(_, inner, _) = ty.kind() {
                // References remain non-null and aligned even when their
                // referent has zero bytes and needs no allocation.
                let alignment = mirsa_domains::allocation::state::type_alignment(self.tcx, *inner);
                self.assumptions.push(not(eq(
                    &pointer.address,
                    &Term::Constant(0, self.address_type()),
                )));
                if alignment > 1 {
                    self.assumptions.push(ieq(
                        &IntTerm::Binary(
                            BinaryOp::Rem,
                            Box::new(IntTerm::FromBits(pointer.address.clone())),
                            Box::new(integer(alignment as i128)),
                        ),
                        &integer(0),
                    ));
                }
                let bytes = match inner.kind() {
                    TyKind::Slice(element) => self
                        .tcx
                        .layout_of(TypingEnv::fully_monomorphized().as_query_input(*element))
                        .ok()
                        .map(|l| l.size.bytes()),
                    TyKind::Str => Some(1),
                    _ => None,
                };
                if let (Some(bytes), Some(length)) = (bytes.filter(|b| *b > 0), &pointer.metadata) {
                    let length = IntTerm::FromBits(length.clone());
                    let end = IntTerm::Binary(
                        BinaryOp::Add,
                        Box::new(pointer.offset.clone()),
                        Box::new(IntTerm::Binary(
                            BinaryOp::Mul,
                            Box::new(length.clone()),
                            Box::new(integer(bytes as i128)),
                        )),
                    );
                    self.assumptions.push(implies(
                        Formula::MathCompare(Comparison::Gt, length, integer(0)),
                        Formula::All(vec![
                            yes(&pointer.has_object),
                            Formula::MathCompare(
                                Comparison::Ge,
                                pointer.offset.clone(),
                                integer(0),
                            ),
                            Formula::MathCompare(
                                Comparison::Le,
                                end,
                                pointer.object.extent.clone(),
                            ),
                        ]),
                    ));
                }
            }
        }
        Ok(())
    }
}
