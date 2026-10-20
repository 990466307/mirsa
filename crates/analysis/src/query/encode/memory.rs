mod relations;

use super::*;
use mirsa_domains::allocation::{AllocationId, PointerValue, state::is_allocation_pointer};
use rustc_middle::ty::{TyKind, TypingEnv};

#[derive(Clone)]
pub(super) struct Object {
    id: IntTerm,
    base: Term,
    extent: IntTerm,
    live: Term,
    freed: Term,
}

#[derive(Clone)]
pub(super) struct Pointer {
    object: Object,
    offset: IntTerm,
    pub(super) address: Term,
    has_object: Term,
    metadata: Option<Term>,
}

fn yes(term: &Term) -> Formula {
    Formula::Compare(
        Comparison::Eq,
        term.clone(),
        Term::Constant(1, ScalarType::BOOL),
    )
}
fn eq(a: &Term, b: &Term) -> Formula {
    Formula::Compare(Comparison::Eq, a.clone(), b.clone())
}
fn ieq(a: &IntTerm, b: &IntTerm) -> Formula {
    Formula::MathCompare(Comparison::Eq, a.clone(), b.clone())
}
fn implies(a: Formula, b: Formula) -> Formula {
    Formula::Any(vec![Formula::Not(Box::new(a)), b])
}
fn not(a: Formula) -> Formula {
    Formula::Not(Box::new(a))
}
fn integer(n: i128) -> IntTerm {
    IntTerm::Constant(n.to_string())
}
fn range(term: &IntTerm, range: Interval) -> Formula {
    if range.is_empty() {
        return Formula::Literal(false);
    }
    let mut bounds = Vec::new();
    if range.low != i128::MIN {
        bounds.push(Formula::MathCompare(
            Comparison::Ge,
            term.clone(),
            integer(range.low),
        ));
    }
    if range.high != i128::MAX {
        bounds.push(Formula::MathCompare(
            Comparison::Le,
            term.clone(),
            integer(range.high),
        ));
    }
    Formula::All(bounds)
}

impl<'tcx> Encoder<'_, 'tcx> {
    pub(super) fn address_type(&self) -> ScalarType {
        ScalarType {
            bits: self.tcx.data_layout.pointer_size.bits() as u32,
            signed: false,
        }
    }
    pub(super) fn fresh_term(&mut self, ty: ScalarType) -> Result<Term> {
        Ok(Term::Variable(self.fresh()?, ty))
    }
    fn object(&mut self, id: Option<&AllocationId>) -> Result<Object> {
        let fact = id.and_then(|id| self.state.allocation.object(id));
        if fact.is_some_and(|f| f.multiplicity.is_exactly_one()) {
            if let Some((_, object)) = self.objects.iter().find(|(key, _)| key.as_ref() == id) {
                return Ok(object.clone());
            }
        }
        let object = Object {
            id: IntTerm::Variable(self.fresh()?),
            base: self.fresh_term(self.address_type())?,
            extent: IntTerm::Variable(self.fresh()?),
            live: self.fresh_term(ScalarType::BOOL)?,
            freed: self.fresh_term(ScalarType::BOOL)?,
        };
        self.assumptions.push(Formula::MathCompare(
            Comparison::Gt,
            object.id.clone(),
            integer(0),
        ));
        self.assumptions.push(not(Formula::All(vec![
            yes(&object.live),
            yes(&object.freed),
        ])));
        if let Some(fact) = fact {
            if fact.alignment > 1 {
                self.assumptions.push(ieq(
                    &IntTerm::Binary(
                        BinaryOp::Rem,
                        Box::new(IntTerm::FromBits(object.base.clone())),
                        Box::new(integer(fact.alignment as i128)),
                    ),
                    &integer(0),
                ));
            }
            if !fact.extent.is_empty() {
                self.assumptions.push(range(&object.extent, fact.extent));
            }
            let mut alternatives = Vec::new();
            if fact.status.may_be_live() {
                alternatives.push(yes(&object.live));
            }
            if fact.status.may_be_dead() {
                alternatives.push(yes(&object.freed));
            }
            if fact.status.may_be_absent() {
                alternatives.push(Formula::All(vec![
                    not(yes(&object.live)),
                    not(yes(&object.freed)),
                ]));
            }
            self.assumptions.push(Formula::Any(alternatives));
        }
        for (other_id, other) in &self.objects {
            let same = ieq(&object.id, &other.id);
            if id
                .zip(other_id.as_ref())
                .is_some_and(|(a, b)| !a.may_alias(b))
            {
                self.assumptions.push(not(same));
            } else {
                self.assumptions.push(implies(
                    same,
                    Formula::All(vec![
                        eq(&object.base, &other.base),
                        ieq(&object.extent, &other.extent),
                        eq(&object.live, &other.live),
                        eq(&object.freed, &other.freed),
                    ]),
                ));
            }
        }
        self.objects.push((id.cloned(), object.clone()));
        Ok(object)
    }

    pub(super) fn is_pointer(&self, expr: &Expr<'tcx>) -> bool {
        matches!(value::ty(self.tcx, self.body, expr), ValueType::Rust(t) if is_allocation_pointer(self.tcx, t))
    }

    fn new_pointer(&mut self, value: &PointerValue, address: Term) -> Result<Pointer> {
        let exact = value.exact_target();
        let p = Pointer {
            object: self.object(exact.map(|(id, _)| id))?,
            offset: match exact {
                Some((_, offset)) if offset.low == offset.high => integer(offset.low),
                _ => IntTerm::Variable(self.fresh()?),
            },
            address,
            has_object: if exact.is_some() {
                Term::Constant(1, ScalarType::BOOL)
            } else {
                self.fresh_term(ScalarType::BOOL)?
            },
            metadata: None,
        };
        self.assumptions.push(implies(
            yes(&p.has_object),
            not(eq(&p.address, &Term::Constant(0, self.address_type()))),
        ));
        Ok(p)
    }

    fn link_address(&mut self, p: &Pointer) {
        let Term::Variable(id, _) = p.address else {
            return;
        };
        if !self.linked_addresses.insert(id) {
            return;
        }
        let sum = IntTerm::Binary(
            BinaryOp::Add,
            Box::new(IntTerm::FromBits(p.object.base.clone())),
            Box::new(p.offset.clone()),
        );
        self.assumptions.push(implies(
            yes(&p.has_object),
            eq(
                &p.address,
                &Term::FromInteger(Box::new(sum), self.address_type()),
            ),
        ));
    }

    pub(super) fn address(&mut self, expr: &Expr<'tcx>) -> Result<Term> {
        if !self.memory_required {
            return self.scalar_address(expr);
        }
        let pointer = self.pointer(expr)?;
        self.link_address(&pointer);
        self.address_relations(expr, &pointer.address)?;
        Ok(pointer.address)
    }

    pub(super) fn pointer(&mut self, expr: &Expr<'tcx>) -> Result<Pointer> {
        self.tick()?;
        let key = self.key(expr);
        if let Some(pointer) = self.pointers.get(&key) {
            return Ok(pointer.clone());
        }
        let abstract_value = match self.eval().eval(expr) {
            Value::Pointer(value) => value,
            _ => PointerValue::top(),
        };
        let address = self.address_variable(expr)?;
        let mut p = self.new_pointer(&abstract_value, address)?;
        self.pointers.insert(key.clone(), p.clone());
        let mut base_source = None;
        if let Expr::TypedExpr(t) = expr {
            if let TypedKind::Builtin(Builtin::Base, args) = &t.kind {
                if self.is_pointer(&args[0]) {
                    base_source = Some(self.pointer(&args[0])?);
                }
            }
        }
        if let Some(source) = base_source {
            self.assumptions.push(implies(
                yes(&source.has_object),
                Formula::All(vec![
                    yes(&p.has_object),
                    ieq(&p.object.id, &source.object.id),
                    ieq(&p.offset, &integer(0)),
                    eq(&p.address, &source.object.base),
                ]),
            ));
        } else {
            let zero = Term::Constant(0, self.address_type());
            let mut alternatives = Vec::new();
            if abstract_value.may_be_null() {
                alternatives.push(Formula::All(vec![
                    eq(&p.address, &zero),
                    not(yes(&p.has_object)),
                ]));
            }
            if abstract_value.may_be_invalid_non_null() {
                alternatives.push(Formula::All(vec![
                    not(eq(&p.address, &zero)),
                    not(yes(&p.has_object)),
                ]));
            }
            if abstract_value.has_unknown_target() {
                alternatives.push(not(eq(&p.address, &zero)));
            }
            for (id, offset) in abstract_value.targets() {
                let object = self.object(Some(id))?;
                alternatives.push(Formula::All(vec![
                    yes(&p.has_object),
                    ieq(&p.object.id, &object.id),
                    range(&p.offset, *offset),
                ]));
            }
            self.assumptions.push(Formula::Any(alternatives));
        }
        if let Some(address) = self.eval().address_place(expr) {
            self.link_address(&p);
            let term = self.variable(address, false, 0)?;
            if term.scalar_type() == Some(self.address_type()) {
                self.assumptions.push(eq(&p.address, &term));
            }
        }
        if let ValueType::Rust(ty) = value::ty(self.tcx, self.body, expr) {
            if self
                .tcx
                .layout_of(TypingEnv::fully_monomorphized().as_query_input(ty))
                .is_ok_and(|l| l.size != self.tcx.data_layout.pointer_size)
            {
                p.metadata = Some(if let Some(place) = self.eval().place(expr) {
                    match ty.builtin_deref(true).map(|t| t.kind()) {
                        Some(TyKind::Slice(_) | TyKind::Str) => self.variable(place, true, 0)?,
                        _ => self.fresh_term(self.address_type())?,
                    }
                } else {
                    self.fresh_term(self.address_type())?
                });
            }
        }
        self.pointers.insert(key, p.clone());
        self.pointer_relations(expr, &p)?;
        Ok(p)
    }

    pub(super) fn pointer_compare(
        &mut self,
        op: Comparison,
        a: &Expr<'tcx>,
        b: &Expr<'tcx>,
    ) -> Result<Formula> {
        self.address(a)?;
        self.address(b)?;
        let a = self.pointer(a)?;
        let b = self.pointer(b)?;
        let mut equality = vec![eq(&a.address, &b.address)];
        match (&a.metadata, &b.metadata) {
            (Some(a), Some(b)) => equality.push(eq(a, b)),
            (None, None) => {}
            _ => return Err(UnknownReason::Unsupported),
        }
        let equality = Formula::All(equality);
        match op {
            Comparison::Eq => Ok(equality),
            Comparison::Ne => Ok(not(equality)),
            _ => Err(UnknownReason::Unsupported),
        }
    }

    pub(super) fn builtin(
        &mut self,
        expr: &Expr<'tcx>,
        op: Builtin,
        args: &[Expr<'tcx>],
        depth: usize,
    ) -> Result<Term> {
        if let Value::Bool(Some(b)) = self.eval().eval(expr) {
            return Ok(Term::Constant(b as u128, ScalarType::BOOL));
        }
        let formula = match op {
            Builtin::IsNan => Formula::IsNan(self.float(&args[0], depth)?),
            Builtin::IsInfinite => Formula::IsInfinite(self.float(&args[0], depth)?),
            Builtin::IsNull => eq(
                &self.address(&args[0])?,
                &Term::Constant(0, self.address_type()),
            ),
            Builtin::IsLive | Builtin::IsFreed | Builtin::SameAllocation => {
                let p = self.pointer(&args[0])?;
                match op {
                    Builtin::IsNull => eq(
                        &self.address(&args[0])?,
                        &Term::Constant(0, self.address_type()),
                    ),
                    Builtin::IsLive => Formula::All(vec![yes(&p.has_object), yes(&p.object.live)]),
                    Builtin::IsFreed => {
                        Formula::All(vec![yes(&p.has_object), yes(&p.object.freed)])
                    }
                    Builtin::SameAllocation => {
                        let other = self.pointer(&args[1])?;
                        Formula::All(vec![
                            yes(&p.has_object),
                            yes(&other.has_object),
                            ieq(&p.object.id, &other.object.id),
                        ])
                    }
                    _ => unreachable!(),
                }
            }
            Builtin::Len => {
                let place = self
                    .eval()
                    .place(&args[0])
                    .ok_or(UnknownReason::Unsupported)?;
                return self.variable(place, true, depth);
            }
            Builtin::Discriminant => return self.scalar_observation(expr),
            _ => return Err(UnknownReason::Unsupported),
        };
        Ok(Term::Boolean(Box::new(formula)))
    }

    pub(super) fn integer_builtin(
        &mut self,
        expr: &Expr<'tcx>,
        op: Builtin,
        args: &[Expr<'tcx>],
    ) -> Result<IntTerm> {
        if let Some(value) = self.eval().eval(expr).exact() {
            return Ok(IntTerm::Constant(value.to_string()));
        }
        match op {
            Builtin::Math => self.integer(&args[0]),
            Builtin::Address => Ok(IntTerm::FromBits(self.address(&args[0])?)),
            Builtin::Offset => Ok(self.pointer(&args[0])?.offset),
            Builtin::Extent => Ok(self.pointer(&args[0])?.object.extent),
            Builtin::LayoutSize | Builtin::LayoutAlign => {
                let key = format!("{op:?}:{}", self.key(&args[0]));
                if let Some(term) = self.integers.get(&key) {
                    return Ok(term.clone());
                }
                let term = IntTerm::Variable(self.fresh()?);
                if let Some((lo, hi)) = self.eval().eval(expr).integer() {
                    self.assumptions.push(Formula::MathCompare(
                        Comparison::Ge,
                        term.clone(),
                        IntTerm::Constant(lo.to_string()),
                    ));
                    self.assumptions.push(Formula::MathCompare(
                        Comparison::Le,
                        term.clone(),
                        IntTerm::Constant(hi.to_string()),
                    ));
                }
                self.integers.insert(key, term.clone());
                Ok(term)
            }
            _ => Err(UnknownReason::Unsupported),
        }
    }

    fn scalar_observation(&mut self, expr: &Expr<'tcx>) -> Result<Term> {
        let ty = abstract_query::expr_type(self.tcx, self.body, expr)
            .ok_or(UnknownReason::Unsupported)?;
        let key = match expr {
            Expr::TypedExpr(t) => match &t.kind {
                TypedKind::Builtin(op, args) => format!("{op:?}:{}", self.key(&args[0])),
                _ => self.key(expr),
            },
            _ => self.key(expr),
        };
        if let Some(term) = self.observations.get(&key) {
            return Ok(term.clone());
        }
        let term = self.fresh_term(ty)?;
        if let Value::Integer(lo, hi) = self.eval().eval(expr) {
            self.assumptions.push(Formula::MathCompare(
                Comparison::Ge,
                IntTerm::FromBits(term.clone()),
                IntTerm::Constant(lo.to_string()),
            ));
            self.assumptions.push(Formula::MathCompare(
                Comparison::Le,
                IntTerm::FromBits(term.clone()),
                IntTerm::Constant(hi.to_string()),
            ));
        }
        self.observations.insert(key, term.clone());
        Ok(term)
    }
}
