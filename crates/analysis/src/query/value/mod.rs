mod builtin;
mod numeric;
pub(super) use builtin::builtin;

use super::{BinaryOp, Comparison, Expr, Predicate, UnaryOp, abstract_query};
use crate::reduced_product::CombinedState;
use mirsa_domains::{
    allocation::{AbstractBool, AllocationId, PointerValue, state::is_allocation_pointer},
    interval::{
        Interval,
        float_interval::{self, FloatCmpOp, FloatInterval},
        transfer::float_kind_for_ty,
    },
};
use mirsa_dsl::Builtin;
use num_bigint::BigInt;
use num_traits::{One, ToPrimitive, Zero};
use rustc_middle::{
    mir::{Body, Operand, Place, ProjectionElem},
    ty::{Ty, TyCtxt, TyKind, TypingEnv},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ValueType<'tcx> {
    Rust(Ty<'tcx>),
    Math,
}

#[derive(Clone, Debug)]
pub(super) struct TypedExpr<'tcx> {
    pub ty: ValueType<'tcx>,
    pub kind: TypedKind<'tcx>,
}

#[derive(Clone, Debug)]
pub(super) enum TypedKind<'tcx> {
    Integer(BigInt),
    Float(f64),
    Boolean(Box<Predicate<'tcx>>),
    Builtin(Builtin, Vec<Expr<'tcx>>),
    Project {
        base: Expr<'tcx>,
        projection: Projection<'tcx>,
    },
}

#[derive(Clone, Debug)]
pub(super) enum Projection<'tcx> {
    Deref,
    Field(usize, Ty<'tcx>),
    Index(Expr<'tcx>),
}

impl<'tcx> TypedExpr<'tcx> {
    pub fn children(&self) -> Vec<&Expr<'tcx>> {
        match &self.kind {
            TypedKind::Builtin(_, args) => args.iter().collect(),
            TypedKind::Project {
                base,
                projection: Projection::Index(index),
            } => vec![base, index],
            TypedKind::Project { base, .. } => vec![base],
            _ => vec![],
        }
    }
    pub fn visit_places(&self, visit: &mut impl FnMut(Place<'tcx>)) {
        for child in self.children() {
            child.visit_places(visit);
        }
        if let TypedKind::Boolean(p) = &self.kind {
            p.visit_places(visit);
        }
    }
}

pub(super) fn typed<'tcx>(ty: ValueType<'tcx>, kind: TypedKind<'tcx>) -> Expr<'tcx> {
    Expr::TypedExpr(Box::new(TypedExpr { ty, kind }))
}

pub(super) fn ty<'tcx>(tcx: TyCtxt<'tcx>, body: &Body<'tcx>, e: &Expr<'tcx>) -> ValueType<'tcx> {
    match e {
        Expr::Value(v) => ValueType::Rust(v.ty(&body.local_decls, tcx)),
        Expr::Constant { ty, .. } | Expr::Cast(_, ty) => ValueType::Rust(*ty),
        Expr::Length(_) => ValueType::Rust(tcx.types.usize),
        Expr::TypedExpr(v) => v.ty,
        Expr::Binary(_, a, _) | Expr::Unary(_, a) => ty(tcx, body, a),
    }
}

#[derive(Clone, Debug)]
pub(super) enum Value {
    Integer(BigInt, BigInt),
    Float(FloatInterval),
    Bool(Option<bool>),
    Pointer(PointerValue),
    Unknown,
}

impl Value {
    pub fn boolean(&self) -> Option<bool> {
        match self {
            Self::Bool(v) => *v,
            _ => None,
        }
    }
    pub fn integer(&self) -> Option<(&BigInt, &BigInt)> {
        match self {
            Self::Integer(a, b) => Some((a, b)),
            _ => None,
        }
    }
    pub fn exact(&self) -> Option<&BigInt> {
        let (a, b) = self.integer()?;
        (a == b).then_some(a)
    }
    pub fn range(value: Interval) -> Self {
        if value.is_empty() {
            Self::Unknown
        } else {
            Self::Integer(value.low.into(), value.high.into())
        }
    }
    pub fn constant(value: impl Into<BigInt>) -> Self {
        let value = value.into();
        Self::Integer(value.clone(), value)
    }
}

pub(super) fn bounds<'tcx>(tcx: TyCtxt<'tcx>, ty: Ty<'tcx>) -> Option<(BigInt, BigInt)> {
    let scalar = abstract_query::scalar_type(tcx, ty)?;
    let bits = scalar.bits;
    Some(if scalar.signed {
        let half = BigInt::one() << (bits - 1);
        (-&half, half - 1)
    } else {
        (BigInt::zero(), (BigInt::one() << bits) - 1)
    })
}

fn raw_integer<'tcx>(tcx: TyCtxt<'tcx>, ty: Ty<'tcx>, bits: u128) -> Value {
    let Some(s) = abstract_query::scalar_type(tcx, ty) else {
        return Value::Unknown;
    };
    if ty.is_bool() {
        return Value::Bool(match bits {
            0 => Some(false),
            1 => Some(true),
            _ => None,
        });
    }
    let mut n = BigInt::from(bits) & ((BigInt::one() << s.bits) - 1);
    if s.signed && n >= (BigInt::one() << (s.bits - 1)) {
        n -= BigInt::one() << s.bits;
    }
    Value::constant(n)
}

pub(super) struct Eval<'a, 'tcx> {
    pub tcx: TyCtxt<'tcx>,
    pub body: &'a Body<'tcx>,
    pub state: &'a CombinedState<'tcx>,
}

impl<'tcx> Eval<'_, 'tcx> {
    pub fn place(&self, expr: &Expr<'tcx>) -> Option<Place<'tcx>> {
        match expr {
            Expr::Value(value) => value.place(),
            Expr::TypedExpr(value) => match &value.kind {
                TypedKind::Project { base, projection } => {
                    let base = self.place(base)?;
                    let p = match projection {
                        Projection::Deref => ProjectionElem::Deref,
                        Projection::Field(index, ty) => ProjectionElem::Field((*index).into(), *ty),
                        Projection::Index(index) => {
                            let v = self.eval(index);
                            let index = v.exact()?.to_u64()?;
                            let length = self.length(base).integer()?.0.to_u64()?;
                            if index >= length {
                                return None;
                            }
                            ProjectionElem::ConstantIndex {
                                offset: index,
                                min_length: index.checked_add(1)?,
                                from_end: false,
                            }
                        }
                    };
                    Some(base.project_deeper(&[p], self.tcx))
                }
                _ => None,
            },
            _ => None,
        }
    }

    fn length(&self, place: Place<'tcx>) -> Value {
        if let TyKind::Array(_, length) = place.ty(&self.body.local_decls, self.tcx).ty.kind() {
            if let Some(n) = length.try_to_target_usize(self.tcx) {
                return Value::constant(n);
            }
        }
        self.state
            .interval
            .tracked_len_resolved(&self.state.symbolic, place)
            .or_else(|| {
                if matches!(place.projection.last(), Some(ProjectionElem::Deref)) {
                    let parent = Place {
                        local: place.local,
                        projection: self
                            .tcx
                            .mk_place_elems(&place.projection[..place.projection.len() - 1]),
                    };
                    self.state
                        .interval
                        .tracked_len_resolved(&self.state.symbolic, parent)
                } else {
                    None
                }
            })
            .map(Value::range)
            .unwrap_or(Value::Unknown)
    }

    fn read(&self, place: Place<'tcx>) -> Value {
        let ty = place.ty(&self.body.local_decls, self.tcx).ty;
        if is_allocation_pointer(self.tcx, ty) {
            return Value::Pointer(
                self.state
                    .allocation
                    .pointer_value_resolved(&self.state.symbolic, place),
            );
        }
        if ty.is_floating_point() {
            return self
                .state
                .interval
                .tracked_float_interval_resolved(&self.state.symbolic, &place)
                .map(Value::Float)
                .unwrap_or(Value::Float(FloatInterval::top()));
        }
        if let Some(bits) = self
            .state
            .interval
            .integer_bits_resolved(&self.state.symbolic, place)
        {
            return raw_integer(self.tcx, ty, bits.to_bits_unchecked());
        }
        let Some((min, max)) = bounds(self.tcx, ty) else {
            return Value::Unknown;
        };
        let (low, high) = if let Some(r) = self
            .state
            .interval
            .tracked_interval_resolved(&self.state.symbolic, &place)
        {
            if r.is_empty() {
                return Value::Unknown;
            }
            if ty == self.tcx.types.u128 {
                if r.low < 0 || r.high == i128::MAX {
                    (min, max)
                } else {
                    (min.max(r.low.into()), max.min(r.high.into()))
                }
            } else {
                (min.max(r.low.into()), max.min(r.high.into()))
            }
        } else {
            (min, max)
        };
        if low > high {
            return Value::Unknown;
        }
        if ty.is_bool() {
            Value::Bool(if low == high {
                Some(!low.is_zero())
            } else {
                None
            })
        } else {
            Value::Integer(low, high)
        }
    }

    pub fn eval(&self, expr: &Expr<'tcx>) -> Value {
        match expr {
            Expr::Value(Operand::Copy(p) | Operand::Move(p)) => self.read(*p),
            Expr::Value(Operand::Constant(c)) => {
                if is_allocation_pointer(self.tcx, c.const_.ty()) {
                    return Value::Pointer(
                        match c
                            .const_
                            .try_eval_scalar_int(self.tcx, TypingEnv::fully_monomorphized())
                        {
                            Some(n) if n.to_bits_unchecked() == 0 => PointerValue::null(),
                            Some(_) => PointerValue::unknown_non_null(),
                            None => PointerValue::top(),
                        },
                    );
                }
                if c.const_.ty().is_floating_point() {
                    let Some(bits) = c
                        .const_
                        .try_eval_scalar_int(self.tcx, TypingEnv::fully_monomorphized())
                        .map(|s| s.to_bits_unchecked())
                    else {
                        return Value::Float(FloatInterval::top());
                    };
                    let f = if c.const_.ty() == self.tcx.types.f32 {
                        f64::from(f32::from_bits(bits as u32))
                    } else {
                        f64::from_bits(bits as u64)
                    };
                    return Value::Float(FloatInterval::singleton(f));
                }
                c.const_
                    .try_eval_scalar_int(self.tcx, TypingEnv::fully_monomorphized())
                    .map(|b| raw_integer(self.tcx, c.const_.ty(), b.to_bits_unchecked()))
                    .unwrap_or(Value::Unknown)
            }
            Expr::Constant { bits, ty } => raw_integer(self.tcx, *ty, *bits),
            Expr::Length(p) => self.length(*p),
            Expr::TypedExpr(v) => match &v.kind {
                TypedKind::Integer(n) => Value::constant(n.clone()),
                TypedKind::Float(f) => Value::Float(FloatInterval::singleton(*f)),
                TypedKind::Boolean(p) => {
                    Value::Bool(abstract_query::evaluate(self.tcx, self.body, self.state, p))
                }
                TypedKind::Project { .. } => self
                    .place(expr)
                    .map(|p| self.read(p))
                    .unwrap_or(Value::Unknown),
                TypedKind::Builtin(op, args) => self.eval_builtin(*op, args),
            },
            Expr::Cast(value, target) => self.cast(self.eval(value), *target),
            Expr::Unary(op, value) => {
                let v = self.eval(value);
                let result = match (op, v) {
                    (UnaryOp::Not, Value::Bool(b)) => Value::Bool(b.map(|v| !v)),
                    (UnaryOp::Neg, Value::Float(f)) => Value::Float(float_interval::neg(&f)),
                    (UnaryOp::Neg, Value::Integer(a, b)) => Value::Integer(-b, -a),
                    (UnaryOp::Not, Value::Integer(a, b)) if a == b => Value::constant(!a),
                    _ => Value::Unknown,
                };
                self.normalize(result, ty(self.tcx, self.body, expr))
            }
            Expr::Binary(op, a, b) => self.binary(*op, a, b),
        }
    }

    pub fn compare(&self, op: Comparison, a: &Expr<'tcx>, b: &Expr<'tcx>) -> Option<bool> {
        let source_ty = ty(self.tcx, self.body, a);
        match (self.eval(a), self.eval(b)) {
            (Value::Integer(al, ah), Value::Integer(bl, bh)) => {
                compare_bounds(op, &al, &ah, &bl, &bh)
            }
            (Value::Bool(a), Value::Bool(b)) => a.zip(b).map(|(a, b)| match op {
                Comparison::Eq => a == b,
                Comparison::Ne => a != b,
                _ => false,
            }),
            (Value::Float(a), Value::Float(b)) => {
                let op = match op {
                    Comparison::Eq => FloatCmpOp::Eq,
                    Comparison::Ne => FloatCmpOp::Ne,
                    Comparison::Lt => FloatCmpOp::Lt,
                    Comparison::Le => FloatCmpOp::Le,
                    Comparison::Gt => FloatCmpOp::Gt,
                    Comparison::Ge => FloatCmpOp::Ge,
                };
                let r = float_interval::compare(op, &a, &b);
                (r.low == r.high).then_some(r.low == 1)
            }
            (Value::Pointer(a), Value::Pointer(b)) => {
                let ValueType::Rust(t) = source_ty else {
                    return None;
                };
                let layout = self
                    .tcx
                    .layout_of(TypingEnv::fully_monomorphized().as_query_input(t))
                    .ok()?;
                if layout.size != self.tcx.data_layout.pointer_size {
                    return None;
                }
                if !matches!(op, Comparison::Eq | Comparison::Ne) {
                    return None;
                }
                let eq = if a.is_null_only() && b.is_null_only() {
                    Some(true)
                } else if a.is_null_only() && b.is_definitely_non_null()
                    || b.is_null_only() && a.is_definitely_non_null()
                {
                    Some(false)
                } else {
                    let (ai, ao) = a.exact_target()?;
                    let (bi, bo) = b.exact_target()?;
                    if ai != bi
                        || !self
                            .state
                            .allocation
                            .object(ai)?
                            .multiplicity
                            .is_exactly_one()
                    {
                        return None;
                    }
                    compare_bounds(
                        Comparison::Eq,
                        &ao.low.into(),
                        &ao.high.into(),
                        &bo.low.into(),
                        &bo.high.into(),
                    )
                };
                eq.map(|eq| if op == Comparison::Eq { eq } else { !eq })
            }
            _ => None,
        }
    }
}

pub(super) fn abstract_bool(value: AbstractBool) -> Option<bool> {
    match value {
        AbstractBool::True => Some(true),
        AbstractBool::False => Some(false),
        _ => None,
    }
}

fn compare_bounds(
    op: Comparison,
    al: &BigInt,
    ah: &BigInt,
    bl: &BigInt,
    bh: &BigInt,
) -> Option<bool> {
    match op {
        Comparison::Eq | Comparison::Ne => {
            let eq = if ah < bl || bh < al {
                Some(false)
            } else if al == ah && bl == bh {
                Some(al == bl)
            } else {
                None
            };
            eq.map(|v| if op == Comparison::Eq { v } else { !v })
        }
        Comparison::Lt => {
            if ah < bl {
                Some(true)
            } else if al >= bh {
                Some(false)
            } else {
                None
            }
        }
        Comparison::Le => {
            if ah <= bl {
                Some(true)
            } else if al > bh {
                Some(false)
            } else {
                None
            }
        }
        Comparison::Gt => compare_bounds(Comparison::Lt, bl, bh, al, ah),
        Comparison::Ge => compare_bounds(Comparison::Le, bl, bh, al, ah),
    }
}

pub(super) fn reduce<'tcx>(
    tcx: TyCtxt<'tcx>,
    body: &Body<'tcx>,
    state: &mut CombinedState<'tcx>,
    property: &Predicate<'tcx>,
) {
    let mut visited = std::collections::HashSet::new();
    property.visit_places(&mut |place| {
        if visited.insert(place) {
            state.reduce_place(tcx, &body.local_decls, place);
        }
    });
    property.visit_exprs(&mut |expr| {
        if let Some(place) = (Eval { tcx, body, state }).place(expr) {
            if visited.insert(place) {
                state.reduce_place(tcx, &body.local_decls, place);
            }
        }
    });
}
