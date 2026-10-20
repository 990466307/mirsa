//! Address projection of pointer facts: no object identity, lifetime or extent variables.
use super::*;
use mirsa_domains::allocation::{AllocationId, PointerValue};
use mirsa_relations::symbolic::SymbolicExpr;
use rustc_middle::ty::{Ty, TyKind};

impl<'tcx> Encoder<'_, 'tcx> {
    pub(super) fn reference_type(&self, expr: &Expr<'tcx>) -> Option<Ty<'tcx>> {
        let mut reference_ty = match value::ty(self.tcx, self.body, expr) {
            ValueType::Rust(ty) if matches!(ty.kind(), TyKind::Ref(..)) => Some(ty),
            _ => None,
        };
        if reference_ty.is_none() {
            if let Some(place) = self.eval().place(expr) {
                reference_ty = self
                    .state
                    .symbolic
                    .equivalent_places(place, mirsa_framework::reduction::MAX_RELATED)
                    .into_iter()
                    .map(|p| p.ty(&self.body.local_decls, self.tcx).ty)
                    .find(|ty| matches!(ty.kind(), TyKind::Ref(..)));
            }
        }
        reference_ty
    }

    pub(super) fn address_variable(&mut self, expr: &Expr<'tcx>) -> Result<Term> {
        let key = self.key(expr);
        if let Some(value) = self.addresses.get(&key) {
            return Ok(value.clone());
        }
        let term = self.fresh_term(self.address_type())?;
        self.addresses.insert(key, term.clone());
        Ok(term)
    }

    fn allocation_address(&mut self, id: &AllocationId) -> Result<Term> {
        let fact = self.state.allocation.object(id);
        let unique = fact.is_some_and(|f| f.multiplicity.is_exactly_one());
        if unique {
            if let Some(value) = self.address_bases.get(id) {
                return Ok(value.clone());
            }
        }
        let base = self.fresh_term(self.address_type())?;
        if let Some(fact) = fact {
            self.address_alignment(&base, fact.alignment);
        }
        if unique {
            self.address_bases.insert(id.clone(), base.clone());
        }
        Ok(base)
    }

    fn address_alignment(&mut self, address: &Term, alignment: u64) {
        if alignment > 1 {
            self.assumptions.push(Formula::MathCompare(
                Comparison::Eq,
                IntTerm::Binary(
                    BinaryOp::Rem,
                    Box::new(IntTerm::FromBits(address.clone())),
                    Box::new(IntTerm::Constant(alignment.to_string())),
                ),
                IntTerm::Constant("0".into()),
            ));
        }
    }

    pub(super) fn scalar_address(&mut self, expr: &Expr<'tcx>) -> Result<Term> {
        let address = self.address_variable(expr)?;
        let Term::Variable(id, _) = address else {
            return Ok(address);
        };
        if !self.scalar_addresses.insert(id) {
            return Ok(address);
        }
        self.tick()?;
        let pointer = match self.eval().eval(expr) {
            Value::Pointer(v) => v,
            _ => PointerValue::top(),
        };
        let zero = Term::Constant(0, self.address_type());
        let eq_zero = Formula::Compare(Comparison::Eq, address.clone(), zero);
        let mut alternatives = Vec::new();
        if pointer.may_be_null() {
            alternatives.push(eq_zero.clone());
        }
        if pointer.may_be_invalid_non_null() || pointer.has_unknown_target() {
            alternatives.push(Formula::Not(Box::new(eq_zero.clone())));
        }
        for (allocation, offset) in pointer.targets() {
            let base = self.allocation_address(allocation)?;
            let offset_term = if offset.low == offset.high {
                IntTerm::Constant(offset.low.to_string())
            } else {
                IntTerm::Variable(self.fresh()?)
            };
            let mut bounds = Vec::new();
            if offset.low != i128::MIN {
                bounds.push(Formula::MathCompare(
                    Comparison::Ge,
                    offset_term.clone(),
                    IntTerm::Constant(offset.low.to_string()),
                ));
            }
            if offset.high != i128::MAX {
                bounds.push(Formula::MathCompare(
                    Comparison::Le,
                    offset_term.clone(),
                    IntTerm::Constant(offset.high.to_string()),
                ));
            }
            let sum = Term::FromInteger(
                Box::new(IntTerm::Binary(
                    BinaryOp::Add,
                    Box::new(IntTerm::FromBits(base)),
                    Box::new(offset_term),
                )),
                self.address_type(),
            );
            bounds.push(Formula::Compare(Comparison::Eq, address.clone(), sum));
            bounds.push(Formula::Not(Box::new(eq_zero.clone())));
            alternatives.push(Formula::All(bounds));
        }
        self.assumptions.push(Formula::Any(alternatives));
        if let Some(place) = self.eval().address_place(expr) {
            let term = self.variable(place, false, 0)?;
            self.assumptions
                .push(Formula::Compare(Comparison::Eq, address.clone(), term));
        }
        if let Some(place) = self.eval().place(expr) {
            if let Some(SymbolicExpr::PointerFromAddress { address: source }) =
                self.state.symbolic.expr_for_place(place).cloned()
            {
                let source = self.operand(&source, 0)?;
                self.assumptions.push(Formula::Compare(
                    Comparison::Eq,
                    address.clone(),
                    Term::Cast(Box::new(source), self.address_type()),
                ));
            }
        }
        self.address_relations(expr, &address)?;
        if let Some(ty) = self.reference_type(expr) {
            if let TyKind::Ref(_, inner, _) = ty.kind() {
                self.assumptions.push(Formula::Not(Box::new(eq_zero)));
                let alignment = mirsa_domains::allocation::state::type_alignment(self.tcx, *inner);
                self.address_alignment(&address, alignment);
            }
        }
        Ok(address)
    }
}
