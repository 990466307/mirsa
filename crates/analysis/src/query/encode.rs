use super::value::{self, Eval, TypedKind, Value, ValueType};
use super::{Expr, Predicate, QueryConfig, abstract_query};
use crate::reduced_product::CombinedState;
use mirsa_domains::interval::Interval;
use mirsa_dsl::Builtin;
use mirsa_framework::access_path::AccessPath;
use mirsa_solver::{
    BinaryOp, Comparison, Formula, IntTerm, Problem, ScalarType, Term, UnaryOp, UnknownReason,
};
use num_traits::ToPrimitive;
use rustc_middle::{
    mir::{Body, Operand, Place, ProjectionElem},
    ty::TyCtxt,
};
use std::collections::{HashMap, HashSet};

mod address;
mod float;
mod integer;
mod memory;
mod projection;
mod state;

type Result<T> = std::result::Result<T, UnknownReason>;

struct Encoder<'a, 'tcx> {
    tcx: TyCtxt<'tcx>,
    body: &'a Body<'tcx>,
    state: &'a CombinedState<'tcx>,
    config: &'a QueryConfig,
    memory_required: bool,
    addresses: HashMap<String, Term>,
    address_bases: HashMap<mirsa_domains::allocation::AllocationId, Term>,
    scalar_addresses: HashSet<u32>,
    variables: HashMap<(AccessPath, ScalarType, bool), Term>,
    pending: Vec<(Place<'tcx>, usize)>,
    queued: HashSet<Place<'tcx>>,
    assumptions: Vec<Formula>,
    nodes: usize,
    next_variable: u32,
    observations: HashMap<String, Term>,
    integers: HashMap<String, IntTerm>,
    pointers: HashMap<String, memory::Pointer>,
    objects: Vec<(
        Option<mirsa_domains::allocation::AllocationId>,
        memory::Object,
    )>,
    arrays: HashMap<(AccessPath, ScalarType), u32>,
    linked_addresses: HashSet<u32>,
    related_addresses: HashSet<u32>,
    predicates: Vec<mirsa_relations::symbolic::PredicateFact<'tcx>>,
}

impl<'tcx> Encoder<'_, 'tcx> {
    fn eval(&self) -> Eval<'_, 'tcx> {
        Eval {
            tcx: self.tcx,
            body: self.body,
            state: self.state,
        }
    }

    fn fresh(&mut self) -> Result<u32> {
        if self.next_variable as usize >= self.config.max_values {
            return Err(UnknownReason::BudgetExceeded);
        }
        let id = self.next_variable;
        self.next_variable += 1;
        Ok(id)
    }

    fn key(&mut self, expr: &Expr<'tcx>) -> String {
        if let Some(place) = self.eval().place(expr) {
            self.queue(place, 0);
            if let Some(path) = self.canonical_path(place) {
                return format!("{path:?}:{:?}", value::ty(self.tcx, self.body, expr));
            }
        }
        format!("{expr:?}")
    }

    fn canonical_path(&self, place: Place<'tcx>) -> Option<AccessPath> {
        if place
            .projection
            .iter()
            .any(|p| matches!(p, ProjectionElem::Index(_)))
        {
            return None;
        }
        let path = self.state.symbolic.normalize_place(place)?;
        self.state.symbolic.eq.members(path).next()
    }
    fn tick(&mut self) -> Result<()> {
        self.nodes += 1;
        if self.nodes > self.config.max_nodes {
            Err(UnknownReason::BudgetExceeded)
        } else {
            Ok(())
        }
    }

    fn queue(&mut self, place: Place<'tcx>, depth: usize) {
        if depth <= self.config.max_depth
            && self.pending.len() < self.config.max_values
            && self.queued.insert(place)
        {
            self.pending.push((place, depth));
        }
    }

    fn variable(&mut self, place: Place<'tcx>, length: bool, depth: usize) -> Result<Term> {
        if place
            .projection
            .iter()
            .any(|p| matches!(p, ProjectionElem::Index(_)))
        {
            return self.indexed_place(place, length, depth);
        }
        let ty = if length {
            self.tcx.types.usize
        } else {
            place.ty(&self.body.local_decls, self.tcx).ty
        };
        let ty = self.storage_type(ty).ok_or(UnknownReason::Unsupported)?;
        let path = self
            .state
            .symbolic
            .normalize_place(place)
            .ok_or(UnknownReason::Unsupported)?;
        let path = if length
            || !place
                .ty(&self.body.local_decls, self.tcx)
                .ty
                .is_floating_point()
        {
            self.state
                .symbolic
                .eq
                .members(path.clone())
                .next()
                .unwrap_or(path)
        } else {
            path
        };
        let key = (path, ty, length);
        self.queue(place, depth);
        if let Some(term) = self.variables.get(&key) {
            return Ok(term.clone());
        }
        if self.variables.len() >= self.config.max_values {
            return Err(UnknownReason::BudgetExceeded);
        }
        let term = Term::Variable(self.fresh()?, ty);
        self.variables.insert(key, term.clone());
        Ok(term)
    }

    fn operand(&mut self, operand: &Operand<'tcx>, depth: usize) -> Result<Term> {
        match operand {
            Operand::Copy(place) | Operand::Move(place) => self.variable(*place, false, depth),
            Operand::Constant(value) => {
                let ty = self
                    .storage_type(value.const_.ty())
                    .ok_or(UnknownReason::Unsupported)?;
                let bits = value
                    .const_
                    .try_eval_scalar_int(
                        self.tcx,
                        rustc_middle::ty::TypingEnv::fully_monomorphized(),
                    )
                    .ok_or(UnknownReason::Unsupported)?
                    .to_bits_unchecked();
                Ok(Term::Constant(bits, ty))
            }
        }
    }

    fn expr(&mut self, expr: &Expr<'tcx>, depth: usize) -> Result<Term> {
        self.tick()?;
        if self.float_bits(expr).is_some() {
            return Ok(Term::FloatBits(Box::new(self.float(expr, depth)?)));
        }
        let result = match expr {
            Expr::TypedExpr(value) => match &value.kind {
                TypedKind::Boolean(p) => Term::Boolean(Box::new(self.property(p)?)),
                TypedKind::Project { .. } => self.projection(expr, depth)?,
                TypedKind::Builtin(op, args) => self.builtin(expr, *op, args, depth)?,
                _ => {
                    let value = (Eval {
                        tcx: self.tcx,
                        body: self.body,
                        state: self.state,
                    })
                    .eval(expr);
                    if let Value::Bool(Some(b)) = value {
                        Term::Constant(u128::from(b), ScalarType::BOOL)
                    } else {
                        return Err(UnknownReason::Unsupported);
                    }
                }
            },
            Expr::Value(value) => self.operand(value, depth)?,
            Expr::Constant { bits, ty } => Term::Constant(
                *bits,
                abstract_query::scalar_type(self.tcx, *ty).ok_or(UnknownReason::Unsupported)?,
            ),
            Expr::Length(place) => {
                if let Some(value) = self.eval().eval(expr).exact().and_then(|n| n.to_u128()) {
                    Term::Constant(
                        value,
                        abstract_query::scalar_type(self.tcx, self.tcx.types.usize).unwrap(),
                    )
                } else {
                    self.variable(*place, true, depth)?
                }
            }
            Expr::Binary(op, a, b) => {
                self.check_defined(*op, a, b)?;
                Term::Binary(
                    *op,
                    Box::new(self.expr(a, depth)?),
                    Box::new(self.expr(b, depth)?),
                )
            }
            Expr::Unary(op, value) => Term::Unary(*op, Box::new(self.expr(value, depth)?)),
            Expr::Cast(value, ty) => {
                let target =
                    abstract_query::scalar_type(self.tcx, *ty).ok_or(UnknownReason::Unsupported)?;
                if self.float_bits(value).is_some() {
                    Term::FloatToInteger(Box::new(self.float(value, depth)?), target)
                } else if value::ty(self.tcx, self.body, value) == ValueType::Math {
                    Term::FromInteger(Box::new(self.integer(value)?), target)
                } else {
                    Term::Cast(Box::new(self.expr(value, depth)?), target)
                }
            }
        };
        result.scalar_type().ok_or(UnknownReason::Unsupported)?;
        Ok(result)
    }

    fn property(&mut self, property: &Predicate<'tcx>) -> Result<Formula> {
        self.tick()?;
        if let Some(value) = abstract_query::evaluate(self.tcx, self.body, self.state, property) {
            return Ok(Formula::Literal(value));
        }
        Ok(match property {
            Predicate::Value(expr) => Formula::Compare(
                Comparison::Eq,
                self.expr(expr, 0)?,
                Term::Constant(1, ScalarType::BOOL),
            ),
            Predicate::Compare(op, a, b) => {
                if self.float_bits(a).is_some() {
                    Formula::FloatCompare(*op, self.float(a, 0)?, self.float(b, 0)?)
                } else if self.is_pointer(a) {
                    self.pointer_compare(*op, a, b)?
                } else if value::ty(self.tcx, self.body, a) == ValueType::Math {
                    Formula::MathCompare(*op, self.integer(a)?, self.integer(b)?)
                } else {
                    Formula::Compare(*op, self.expr(a, 0)?, self.expr(b, 0)?)
                }
            }
            Predicate::Not(value) => Formula::Not(Box::new(self.property(value)?)),
            Predicate::All(values) => Formula::All(
                values
                    .iter()
                    .map(|value| self.property(value))
                    .collect::<Result<_>>()?,
            ),
            Predicate::Any(values) => Formula::Any(
                values
                    .iter()
                    .map(|value| self.property(value))
                    .collect::<Result<_>>()?,
            ),
        })
    }

    fn interval(&mut self, term: &Term, interval: Interval) {
        if interval.is_empty() {
            return;
        }
        let Some(ty) = term.scalar_type() else {
            return;
        };
        let (low, high) = if ty.signed {
            if ty.bits == 128 {
                (i128::MIN, i128::MAX)
            } else {
                (-(1i128 << (ty.bits - 1)), (1i128 << (ty.bits - 1)) - 1)
            }
        } else {
            (
                0,
                if ty.bits >= 127 {
                    i128::MAX
                } else {
                    (1i128 << ty.bits) - 1
                },
            )
        };
        if interval.low > low && interval.low <= high {
            self.assumptions.push(Formula::Compare(
                Comparison::Ge,
                term.clone(),
                Term::Constant(interval.low as u128, ty),
            ));
        }
        if interval.high < high && interval.high >= low {
            self.assumptions.push(Formula::Compare(
                Comparison::Le,
                term.clone(),
                Term::Constant(interval.high as u128, ty),
            ));
        }
    }
}

pub(super) fn problem<'tcx>(
    tcx: TyCtxt<'tcx>,
    body: &Body<'tcx>,
    state: &CombinedState<'tcx>,
    property: &Predicate<'tcx>,
    config: &QueryConfig,
) -> Result<Problem> {
    let mut encoder = Encoder {
        tcx,
        body,
        state,
        config,
        memory_required: super::demand::Demand::predicate(property).memory,
        addresses: HashMap::new(),
        address_bases: HashMap::new(),
        scalar_addresses: HashSet::new(),
        variables: HashMap::new(),
        pending: Vec::new(),
        queued: HashSet::new(),
        assumptions: Vec::new(),
        nodes: 0,
        next_variable: 0,
        observations: HashMap::new(),
        integers: HashMap::new(),
        pointers: HashMap::new(),
        objects: Vec::new(),
        arrays: HashMap::new(),
        linked_addresses: HashSet::new(),
        related_addresses: HashSet::new(),
        predicates: Vec::new(),
    };
    let property = encoder.property(property)?;
    encoder.collect()?;
    Ok(Problem {
        assumptions: encoder.assumptions,
        property,
    })
}
