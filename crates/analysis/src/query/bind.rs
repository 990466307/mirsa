use super::value::{self, Projection, TypedKind, ValueType};
use super::{BinaryOp, Comparison, Expr, Predicate, UnaryOp};
use mirsa_dsl::{self as dsl, Diagnostic, ExprKind};
use num_bigint::BigInt;
use num_traits::{One, ToPrimitive};
use rustc_middle::{
    mir::{Body, Location, Operand, Place},
    ty::{Ty, TyCtxt, TyKind, TypingEnv},
};
use std::collections::HashMap;

#[derive(Clone, Debug)]
pub struct ValueBinding<'tcx>(Operand<'tcx>);
impl<'tcx> From<Place<'tcx>> for ValueBinding<'tcx> {
    fn from(p: Place<'tcx>) -> Self {
        Self(Operand::Copy(p))
    }
}
impl<'tcx> From<Operand<'tcx>> for ValueBinding<'tcx> {
    fn from(p: Operand<'tcx>) -> Self {
        Self(p)
    }
}

#[derive(Clone, Debug, Default)]
pub struct Bindings<'tcx> {
    pub(super) values: HashMap<String, Operand<'tcx>>,
    scoped_values: HashMap<String, HashMap<String, Operand<'tcx>>>,
    types: HashMap<String, Ty<'tcx>>,
    pub(super) locations: HashMap<String, Location>,
}
impl<'tcx> Bindings<'tcx> {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn value(mut self, name: impl Into<String>, value: impl Into<ValueBinding<'tcx>>) -> Self {
        self.values.insert(name.into(), value.into().0);
        self
    }
    pub fn ty(mut self, name: impl Into<String>, ty: Ty<'tcx>) -> Self {
        self.types.insert(name.into(), ty);
        self
    }
    /// Bind a value at one named location, overriding a global value binding.
    pub fn value_at(
        mut self,
        name: impl Into<String>,
        value: impl Into<ValueBinding<'tcx>>,
        at: impl Into<String>,
    ) -> Self {
        self.scoped_values
            .entry(at.into())
            .or_default()
            .insert(name.into(), value.into().0);
        self
    }
    pub(super) fn for_location(&self, at: &str) -> std::borrow::Cow<'_, Self> {
        let Some(values) = self.scoped_values.get(at) else {
            return std::borrow::Cow::Borrowed(self);
        };
        std::borrow::Cow::Owned(Self {
            values: self
                .values
                .iter()
                .chain(values)
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
            types: self.types.clone(),
            locations: self.locations.clone(),
            scoped_values: HashMap::new(),
        })
    }
    pub fn location(mut self, name: impl Into<String>, location: Location) -> Self {
        self.locations.insert(name.into(), location);
        self
    }
}

pub(super) struct Binder<'a, 'tcx> {
    pub tcx: TyCtxt<'tcx>,
    pub body: &'a Body<'tcx>,
    pub bindings: &'a Bindings<'tcx>,
}
type Result<T> = std::result::Result<T, Diagnostic>;

impl<'tcx> Binder<'_, 'tcx> {
    fn error(&self, e: &dsl::Expr, message: impl Into<String>) -> Diagnostic {
        Diagnostic {
            span: e.span.clone(),
            message: message.into(),
        }
    }
    fn scalar(&self, name: &str) -> Ty<'tcx> {
        let t = &self.tcx.types;
        match name {
            "u8" => t.u8,
            "u16" => t.u16,
            "u32" => t.u32,
            "u64" => t.u64,
            "u128" => t.u128,
            "usize" => t.usize,
            "i8" => t.i8,
            "i16" => t.i16,
            "i32" => t.i32,
            "i64" => t.i64,
            "i128" => t.i128,
            "isize" => t.isize,
            "f32" => t.f32,
            "f64" => t.f64,
            "bool" => t.bool,
            "char" => t.char,
            _ => unreachable!(),
        }
    }
    fn hint(&self, e: &dsl::Expr) -> Option<ValueType<'tcx>> {
        match &e.kind {
            ExprKind::Number(n) => number_parts(n)
                .ok()
                .and_then(|(_, s, _)| s.map(|s| ValueType::Rust(self.scalar(s)))),
            ExprKind::Unary(dsl::Unary::Neg | dsl::Unary::Not, e) => self.hint(e),
            ExprKind::Binary(op, a, b)
                if !matches!(
                    op,
                    dsl::Binary::Eq
                        | dsl::Binary::Ne
                        | dsl::Binary::Lt
                        | dsl::Binary::Le
                        | dsl::Binary::Gt
                        | dsl::Binary::Ge
                        | dsl::Binary::And
                        | dsl::Binary::Or
                        | dsl::Binary::Implies
                ) =>
            {
                self.hint(a).or_else(|| self.hint(b))
            }
            _ => self
                .bind(e, None)
                .ok()
                .map(|e| value::ty(self.tcx, self.body, &e)),
        }
    }
    fn require(&self, e: &dsl::Expr, ok: bool, message: &str) -> Result<()> {
        if ok {
            Ok(())
        } else {
            Err(self.error(e, message))
        }
    }
    fn is_integer(&self, t: ValueType<'tcx>) -> bool {
        matches!(t, ValueType::Math) || matches!(t,ValueType::Rust(t) if t.is_integral())
    }
    fn is_float(&self, t: ValueType<'tcx>) -> bool {
        matches!(t,ValueType::Rust(t) if t.is_floating_point())
    }
    fn is_bool(&self, t: ValueType<'tcx>) -> bool {
        t == ValueType::Rust(self.tcx.types.bool)
    }
    fn is_pointer(&self, t: ValueType<'tcx>) -> bool {
        matches!(t,ValueType::Rust(t) if mirsa_domains::allocation::state::is_allocation_pointer(self.tcx,t))
    }
    fn boolean(&self, p: Predicate<'tcx>) -> Expr<'tcx> {
        value::typed(
            ValueType::Rust(self.tcx.types.bool),
            TypedKind::Boolean(Box::new(p)),
        )
    }
    fn predicate(&self, e: Expr<'tcx>) -> Predicate<'tcx> {
        match e {
            Expr::TypedExpr(v) => match v.kind {
                TypedKind::Boolean(p) => *p,
                kind => Predicate::Value(value::typed(v.ty, kind)),
            },
            e => Predicate::Value(e),
        }
    }
    pub fn bind(&self, e: &dsl::Expr, expected: Option<ValueType<'tcx>>) -> Result<Expr<'tcx>> {
        let out = self.lower(e, expected)?;
        if let Some(expected) = expected {
            self.require(
                e,
                value::ty(self.tcx, self.body, &out) == expected,
                "expression has an incompatible type",
            )?;
        }
        Ok(out)
    }
    fn lower(&self, e: &dsl::Expr, expected: Option<ValueType<'tcx>>) -> Result<Expr<'tcx>> {
        let expr = match &e.kind {
            ExprKind::Binding(name) => {
                let v = self
                    .bindings
                    .values
                    .get(name)
                    .ok_or_else(|| self.error(e, format!("unknown value binding ${name}")))?;
                if let Some(p) = v.place() {
                    self.require(
                        e,
                        p.local.index() < self.body.local_decls.len(),
                        "binding belongs to a different MIR body",
                    )?;
                }
                Expr::Value(v.clone())
            }
            ExprKind::Number(n) => return self.number(e, n, expected, false),
            ExprKind::Bool(b) => Expr::Constant {
                bits: u128::from(*b),
                ty: self.tcx.types.bool,
            },
            ExprKind::Unary(dsl::Unary::Neg, a) if matches!(a.kind, ExprKind::Number(_)) => {
                let ExprKind::Number(n) = &a.kind else {
                    unreachable!()
                };
                return self.number(e, n, expected, true);
            }
            ExprKind::Unary(op, a) => {
                let a = self.bind(
                    a,
                    if *op == dsl::Unary::Deref {
                        None
                    } else {
                        expected
                    },
                )?;
                let t = value::ty(self.tcx, self.body, &a);
                match op {
                    dsl::Unary::Deref => {
                        let ValueType::Rust(t) = t else {
                            return Err(self.error(e, "dereference requires a pointer"));
                        };
                        let inner = match t.kind() {
                            TyKind::Ref(_, t, _) | TyKind::RawPtr(t, _) => *t,
                            _ => {
                                return Err(self
                                    .error(e, "dereference requires a raw pointer or reference"));
                            }
                        };
                        value::typed(
                            ValueType::Rust(inner),
                            TypedKind::Project {
                                base: a,
                                projection: Projection::Deref,
                            },
                        )
                    }
                    dsl::Unary::Neg => {
                        self.require(
                            e,
                            self.is_integer(t) || self.is_float(t),
                            "negation requires a number",
                        )?;
                        Expr::Unary(UnaryOp::Neg, Box::new(a))
                    }
                    dsl::Unary::Not => {
                        self.require(
                            e,
                            self.is_integer(t) || self.is_bool(t),
                            "! requires an integer or bool",
                        )?;
                        if self.is_bool(t) {
                            self.boolean(Predicate::Not(Box::new(self.predicate(a))))
                        } else {
                            Expr::Unary(UnaryOp::Not, Box::new(a))
                        }
                    }
                }
            }
            ExprKind::Binary(op, a, b) => {
                use dsl::Binary::*;
                if matches!(op, And | Or | Implies) {
                    let t = ValueType::Rust(self.tcx.types.bool);
                    let a = self.bind(a, Some(t))?;
                    let b = self.bind(b, Some(t))?;
                    let (a, b) = (self.predicate(a), self.predicate(b));
                    return Ok(self.boolean(match op {
                        And => Predicate::All(vec![a, b]),
                        Or => Predicate::Any(vec![a, b]),
                        _ => Predicate::Any(vec![Predicate::Not(Box::new(a)), b]),
                    }));
                }
                let compare = matches!(op, Eq | Ne | Lt | Le | Gt | Ge);
                let t =
                    self.hint(a)
                        .or_else(|| self.hint(b))
                        .or(if compare { None } else { expected });
                let a = self.bind(a, t)?;
                let at = value::ty(self.tcx, self.body, &a);
                let shift = matches!(op, Shl | Shr);
                let b = self.bind(b, if shift { None } else { Some(at) })?;
                let bt = value::ty(self.tcx, self.body, &b);
                if compare {
                    self.require(e, at == bt, "comparison operands have different types")?;
                    self.require(
                        e,
                        self.is_integer(at)
                            || self.is_float(at)
                            || matches!(at,ValueType::Rust(t) if t.is_char())
                            || (matches!(op, Eq | Ne) && (self.is_bool(at) || self.is_pointer(at))),
                        "unsupported comparison operand type",
                    )?;
                    self.boolean(Predicate::Compare(
                        match op {
                            Eq => Comparison::Eq,
                            Ne => Comparison::Ne,
                            Lt => Comparison::Lt,
                            Le => Comparison::Le,
                            Gt => Comparison::Gt,
                            _ => Comparison::Ge,
                        },
                        a,
                        b,
                    ))
                } else {
                    let bit = matches!(op, BitAnd | BitOr | BitXor | Shl | Shr);
                    self.require(
                        e,
                        if shift {
                            self.is_integer(at) && self.is_integer(bt)
                        } else {
                            at == bt
                                && (self.is_integer(at)
                                    || if bit {
                                        self.is_bool(at)
                                    } else {
                                        self.is_float(at)
                                    })
                        },
                        "invalid arithmetic operand types",
                    )?;
                    Expr::Binary(
                        match op {
                            Add => BinaryOp::Add,
                            Sub => BinaryOp::Sub,
                            Mul => BinaryOp::Mul,
                            Div => BinaryOp::Div,
                            Rem => BinaryOp::Rem,
                            BitAnd => BinaryOp::And,
                            BitOr => BinaryOp::Or,
                            BitXor => BinaryOp::Xor,
                            Shl => BinaryOp::Shl,
                            Shr => BinaryOp::Shr,
                            _ => unreachable!(),
                        },
                        Box::new(a),
                        Box::new(b),
                    )
                }
            }
            ExprKind::Cast(a, name) => {
                let a = self.bind(a, None)?;
                let t = value::ty(self.tcx, self.body, &a);
                let to = self.scalar(name);
                let from_char = matches!(t,ValueType::Rust(t) if t.is_char());
                self.require(
                    e,
                    if to.is_char() {
                        t == ValueType::Rust(self.tcx.types.u8)
                    } else {
                        (to.is_integral()
                            && (self.is_integer(t)
                                || self.is_float(t)
                                || self.is_bool(t)
                                || from_char))
                            || (to.is_floating_point() && (self.is_integer(t) || self.is_float(t)))
                    },
                    "unsupported scalar cast",
                )?;
                Expr::Cast(Box::new(a), to)
            }
            ExprKind::Field(base, name) => {
                let base = self.bind(base, None)?;
                let ValueType::Rust(t) = value::ty(self.tcx, self.body, &base) else {
                    return Err(self.error(e, "field access requires an aggregate"));
                };
                let (index,field_ty)=match t.kind(){
                    TyKind::Tuple(fields)=>{let i=name.parse::<usize>().map_err(|_|self.error(e,"tuple fields use numeric indices"))?;let t=fields.get(i).ok_or_else(||self.error(e,"tuple field out of range"))?;(i,*t)},
                    TyKind::Adt(adt,args) if !adt.is_enum()=>{let fields=&adt.non_enum_variant().fields;let i=name.parse::<usize>().ok().or_else(||fields.iter().position(|f|f.name.as_str()==name)).ok_or_else(||self.error(e,"unknown field"))?;let f=fields.get(i.into()).ok_or_else(||self.error(e,"field out of range"))?;(i,f.ty(self.tcx,args))},
                    _=>return Err(self.error(e,"field access requires a tuple or struct; bind an enum projection explicitly")),
                };
                value::typed(
                    ValueType::Rust(field_ty),
                    TypedKind::Project {
                        base,
                        projection: Projection::Field(index, field_ty),
                    },
                )
            }
            ExprKind::Index(base, index) => {
                let mut base = self.bind(base, None)?;
                let ValueType::Rust(mut t) = value::ty(self.tcx, self.body, &base) else {
                    return Err(self.error(e, "index requires an array or slice"));
                };
                if let TyKind::Ref(_, inner, _) = t.kind() {
                    t = *inner;
                    base = value::typed(
                        ValueType::Rust(t),
                        TypedKind::Project {
                            base,
                            projection: Projection::Deref,
                        },
                    );
                }
                let inner = match t.kind() {
                    TyKind::Array(t, _) | TyKind::Slice(t) => *t,
                    _ => return Err(self.error(e, "index requires an array or slice")),
                };
                let index = self.bind(
                    index,
                    self.hint(index)
                        .or(Some(ValueType::Rust(self.tcx.types.usize))),
                )?;
                self.require(
                    e,
                    self.is_integer(value::ty(self.tcx, self.body, &index)),
                    "index must be an integer",
                )?;
                value::typed(
                    ValueType::Rust(inner),
                    TypedKind::Project {
                        base,
                        projection: Projection::Index(index),
                    },
                )
            }
            ExprKind::Builtin(op, args) => {
                let args = args
                    .iter()
                    .map(|e| self.bind(e, None))
                    .collect::<Result<Vec<_>>>()?;
                value::builtin(self.tcx, self.body, *op, args)
                    .map_err(|message| self.error(e, message))?
            }
            ExprKind::TypeLayout { align, ty } => {
                let t = if ty.binding {
                    *self.bindings.types.get(&ty.name).ok_or_else(|| {
                        self.error(e, format!("unknown type binding ${}", ty.name))
                    })?
                } else {
                    self.scalar(&ty.name)
                };
                let layout = self
                    .tcx
                    .layout_of(TypingEnv::fully_monomorphized().as_query_input(t))
                    .map_err(|_| self.error(e, "type layout is not statically known"))?;
                self.require(e, layout.is_sized(), "type layout requires a Sized type")?;
                value::typed(
                    ValueType::Math,
                    TypedKind::Integer(
                        if *align {
                            layout.align.abi.bytes()
                        } else {
                            layout.size.bytes()
                        }
                        .into(),
                    ),
                )
            }
        };
        Ok(expr)
    }

    fn number(
        &self,
        e: &dsl::Expr,
        text: &str,
        expected: Option<ValueType<'tcx>>,
        negative: bool,
    ) -> Result<Expr<'tcx>> {
        let (raw, suffix, float) = number_parts(text).map_err(|m| self.error(e, m))?;
        self.require(e, raw.len() <= 4096, "numeric literal exceeds budget")?;
        let ty = suffix
            .map(|s| ValueType::Rust(self.scalar(s)))
            .or(expected)
            .unwrap_or(ValueType::Rust(self.tcx.types.i32));
        if float {
            self.require(
                e,
                self.is_float(ty),
                "float literal requires f32 or f64 suffix",
            )?;
            let ValueType::Rust(t) = ty else {
                unreachable!()
            };
            // Round directly to the target type. Parsing through f64 first
            // can double-round decimal literals near an f32 midpoint.
            let number = if t == self.tcx.types.f32 {
                raw.parse::<f32>().map(f64::from)
            } else {
                raw.parse::<f64>()
            }
            .map_err(|_| self.error(e, "invalid float literal"))?;
            let number = if negative { -number } else { number };
            if let Some(expected) = expected {
                self.require(e, ty == expected, "literal type mismatch")?;
            }
            return Ok(value::typed(ty, TypedKind::Float(number)));
        }
        self.require(
            e,
            self.is_integer(ty),
            "integer literal requires an integer type",
        )?;
        let (digits, radix) = raw.strip_prefix("0x").map_or((raw, 10), |n| (n, 16));
        let mut n = BigInt::parse_bytes(digits.as_bytes(), radix)
            .ok_or_else(|| self.error(e, "invalid integer literal"))?;
        if negative {
            n = -n;
        }
        if n.bits() > 8192 {
            return Err(self.error(e, "integer literal exceeds budget"));
        }
        if let Some(expected) = expected {
            self.require(e, ty == expected, "literal type mismatch")?;
        }
        match ty {
            ValueType::Math => Ok(value::typed(ty, TypedKind::Integer(n))),
            ValueType::Rust(t) => {
                let (low, high) = value::bounds(self.tcx, t)
                    .ok_or_else(|| self.error(e, "invalid integer type"))?;
                self.require(e, n >= low && n <= high, "integer literal is out of range")?;
                let bits = if n < 0.into() {
                    n + (BigInt::one()
                        << super::abstract_query::scalar_type(self.tcx, t)
                            .unwrap()
                            .bits)
                } else {
                    n
                };
                Ok(Expr::Constant {
                    bits: bits.to_u128().unwrap(),
                    ty: t,
                })
            }
        }
    }
}

fn number_parts(text: &str) -> std::result::Result<(&str, Option<&str>, bool), &'static str> {
    let float = text.contains('.');
    let suffix = [
        "usize", "isize", "u128", "i128", "u64", "i64", "u32", "i32", "u16", "i16", "u8", "i8",
        "f32", "f64",
    ]
    .into_iter()
    .find(|s| text.ends_with(s) && (!text.starts_with("0x") || !s.starts_with("f")));
    let raw = suffix.map_or(text, |s| &text[..text.len() - s.len()]);
    if float && !matches!(suffix, Some("f32" | "f64")) {
        return Err("float literal requires f32 or f64 suffix");
    }
    if !float && matches!(suffix, Some("f32" | "f64")) {
        return Err("float literal requires a decimal point");
    }
    Ok((raw, suffix, float))
}
