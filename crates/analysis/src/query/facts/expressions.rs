use super::super::{Bindings, Diagnostic};
use super::model::*;
use crate::reduced_product::CombinedState;
use mirsa_framework::access_path::{AccessPath, AccessPathElem};
use mirsa_relations::{
    symbolic::{PredicateFact, SymbolicExpr},
    value_expr::{CheckedComponent, ValueExpr},
};
use rustc_middle::{
    mir::{BinOp, Body, Operand, UnOp},
    ty::TyCtxt,
};
use std::collections::HashMap;

pub(super) struct Exporter<'a, 'tcx> {
    pub tcx: TyCtxt<'tcx>,
    pub body: &'a Body<'tcx>,
    names: HashMap<AccessPath, String>,
}
impl<'a, 'tcx> Exporter<'a, 'tcx> {
    pub fn new(
        tcx: TyCtxt<'tcx>,
        body: &'a Body<'tcx>,
        state: Option<&CombinedState<'tcx>>,
        bindings: &Bindings<'tcx>,
    ) -> Self {
        let mut names = HashMap::<AccessPath, String>::new();
        for (name, operand) in &bindings.values {
            if let Some(place) = operand.place() {
                for path in [
                    AccessPath::from_place(place),
                    state.and_then(|s| s.symbolic.normalize_place(place)),
                ]
                .into_iter()
                .flatten()
                {
                    let name = format!("${name}");
                    names
                        .entry(path)
                        .and_modify(|old| {
                            if name < *old {
                                *old = name.clone();
                            }
                        })
                        .or_insert(name);
                }
            }
        }
        Self { tcx, body, names }
    }
    pub fn path(&self, path: &AccessPath) -> Expression {
        if let Some(name) = self.names.get(path) {
            return Expression::variable(name);
        }
        let root = AccessPath::from_local(path.root);
        let mut value = Expression::variable(
            self.names
                .get(&root)
                .cloned()
                .unwrap_or_else(|| root.to_string()),
        );
        for elem in &path.elems {
            let element = match elem {
                AccessPathElem::Deref => Projection::Deref,
                AccessPathElem::Field(n) => Projection::Field(*n),
                AccessPathElem::Downcast(n) => Projection::Downcast(*n),
                AccessPathElem::Index => Projection::AnyIndex,
                AccessPathElem::ConstantIndex { offset, from_end } => Projection::ConstantIndex {
                    offset: *offset,
                    from_end: *from_end,
                },
            };
            value = Expression::Projection {
                base: Box::new(value),
                element,
            };
        }
        value
    }
    pub fn operand(&self, value: &Operand<'tcx>) -> Expression {
        if let Some(path) = value.place().and_then(AccessPath::from_place) {
            return Expression::Typed {
                ty: value.ty(&self.body.local_decls, self.tcx).to_string(),
                expression: Box::new(self.path(&path)),
            };
        }
        let ty = value.ty(&self.body.local_decls, self.tcx).to_string();
        if let Operand::Constant(c) = value {
            if let Some(bits) = c.const_.try_to_scalar_int() {
                return Expression::Constant {
                    bits: bits.to_bits_unchecked().to_string(),
                    width: Some(bits.size().bits()),
                    ty,
                };
            }
        }
        Expression::OpaqueValue {
            identity: format!("{value:?}"),
            ty,
        }
    }
    pub fn constant(&self, bits: u128, ty: rustc_middle::ty::Ty<'tcx>) -> Expression {
        let width = self
            .tcx
            .layout_of(rustc_middle::ty::TypingEnv::fully_monomorphized().as_query_input(ty))
            .ok()
            .map(|l| l.size.bits());
        Expression::Constant {
            bits: bits.to_string(),
            width,
            ty: ty.to_string(),
        }
    }
    pub fn symbolic(&self, value: &SymbolicExpr<'tcx>) -> Expression {
        let operand = |v| Box::new(self.operand(v));
        match value {
            SymbolicExpr::Value(ValueExpr::Use(v)) => self.operand(v),
            SymbolicExpr::Value(ValueExpr::Binary { op, left, right })
            | SymbolicExpr::Cmp { op, left, right } => Expression::Binary {
                operator: binary(*op),
                left: operand(left),
                right: operand(right),
            },
            SymbolicExpr::Value(ValueExpr::CheckedBinary {
                op,
                left,
                right,
                component,
            }) => Expression::CheckedBinary {
                operator: binary(*op),
                overflow: *component == CheckedComponent::Overflow,
                left: operand(left),
                right: operand(right),
            },
            SymbolicExpr::Value(ValueExpr::Unary { op, operand: v }) => Expression::Unary {
                operator: unary(*op),
                operand: operand(v),
            },
            SymbolicExpr::Value(ValueExpr::Cast {
                kind,
                operand: v,
                target,
            }) => Expression::Cast {
                cast_kind: format!("{kind:?}"),
                ty: target.to_string(),
                operand: operand(v),
            },
            SymbolicExpr::Pointer { base, offset: None } => self.operand(base),
            SymbolicExpr::Pointer {
                base,
                offset: Some((op, count)),
            } => Expression::PointerOffset {
                base: operand(base),
                count: operand(count),
                bytes: op.bytes,
                subtract: op.subtract,
                wrapping: op.wrapping,
            },
            SymbolicExpr::PointerFromAddress { address } => Expression::PointerFromAddress {
                address: operand(address),
            },
            SymbolicExpr::Address { pointer } => self.operand(pointer).property(Property::Address),
            SymbolicExpr::Length { place } => self
                .operand(&Operand::Copy(*place))
                .property(Property::Length),
            SymbolicExpr::Call { callee, args } => Expression::Call {
                callee: self.tcx.def_path_str(*callee),
                arguments: args.iter().map(|v| self.operand(v)).collect(),
            },
        }
    }
    pub fn predicate(&self, value: &PredicateFact<'tcx>) -> Result<Expression, Vec<Diagnostic>> {
        let syntax = mirsa_dsl::parse_expression(&value.expression)?;
        self.syntax(&syntax, &|name| {
            value.values.get(name).map(|v| self.operand(v))
        })
    }
    pub fn syntax(
        &self,
        syntax: &mirsa_dsl::Expr,
        binding: &impl Fn(&str) -> Option<Expression>,
    ) -> Result<Expression, Vec<Diagnostic>> {
        use mirsa_dsl::{Binary as B, ExprKind as K, Unary as U};
        let recurse = |e| self.syntax(e, binding).map(Box::new);
        Ok(match &syntax.kind {
            K::Binding(name) => binding(name).ok_or_else(|| {
                vec![Diagnostic {
                    span: syntax.span.clone(),
                    message: format!("missing symbolic binding ${name}"),
                }]
            })?,
            K::Number(n) => Expression::Literal { token: n.clone() },
            K::Bool(v) => Expression::Literal {
                token: v.to_string(),
            },
            K::Unary(op, v) => Expression::Unary {
                operator: match op {
                    U::Not => UnaryOperator::Not,
                    U::Neg => UnaryOperator::Neg,
                    U::Deref => UnaryOperator::Deref,
                },
                operand: recurse(v)?,
            },
            K::Binary(op, a, b) => Expression::Binary {
                operator: match op {
                    B::Implies => BinaryOperator::Implies,
                    B::Or => BinaryOperator::LogicalOr,
                    B::And => BinaryOperator::LogicalAnd,
                    B::Eq => BinaryOperator::Eq,
                    B::Ne => BinaryOperator::Ne,
                    B::Lt => BinaryOperator::Lt,
                    B::Le => BinaryOperator::Le,
                    B::Gt => BinaryOperator::Gt,
                    B::Ge => BinaryOperator::Ge,
                    B::BitOr => BinaryOperator::BitOr,
                    B::BitXor => BinaryOperator::BitXor,
                    B::BitAnd => BinaryOperator::BitAnd,
                    B::Shl => BinaryOperator::Shl,
                    B::Shr => BinaryOperator::Shr,
                    B::Add => BinaryOperator::Add,
                    B::Sub => BinaryOperator::Sub,
                    B::Mul => BinaryOperator::Mul,
                    B::Div => BinaryOperator::Div,
                    B::Rem => BinaryOperator::Rem,
                },
                left: recurse(a)?,
                right: recurse(b)?,
            },
            K::Cast(v, ty) => Expression::Cast {
                cast_kind: "dsl".into(),
                ty: ty.clone(),
                operand: recurse(v)?,
            },
            K::Field(v, field) => Expression::Projection {
                base: recurse(v)?,
                element: field
                    .parse()
                    .map(Projection::Field)
                    .unwrap_or_else(|_| Projection::NamedField(field.clone())),
            },
            K::Index(v, i) => Expression::Index {
                base: recurse(v)?,
                index: recurse(i)?,
            },
            K::Builtin(op, args) => {
                let args = args
                    .iter()
                    .map(|v| self.syntax(v, binding))
                    .collect::<Result<Vec<_>, _>>()?;
                if let Some(property) = property(*op) {
                    args[0].clone().property(property)
                } else {
                    Expression::Call {
                        callee: "same_allocation".into(),
                        arguments: args,
                    }
                }
            }
            K::TypeLayout { align, ty } => Expression::TypeLayout {
                ty: if ty.binding {
                    format!("${}", ty.name)
                } else {
                    ty.name.clone()
                },
                alignment: *align,
            },
        })
    }
}

pub(super) fn property(op: mirsa_dsl::Builtin) -> Option<Property> {
    use mirsa_dsl::Builtin as B;
    Some(match op {
        B::Math => Property::Math,
        B::Len => Property::Length,
        B::IsNan => Property::IsNan,
        B::IsInfinite => Property::IsInfinite,
        B::IsNull => Property::IsNull,
        B::IsLive => Property::IsLive,
        B::IsFreed => Property::IsFreed,
        B::Base => Property::Base,
        B::Address => Property::Address,
        B::Offset => Property::Offset,
        B::Extent => Property::Extent,
        B::LayoutSize => Property::LayoutSize,
        B::LayoutAlign => Property::LayoutAlign,
        B::Discriminant => Property::Discriminant,
        B::SameAllocation => return None,
    })
}
fn binary(op: BinOp) -> BinaryOperator {
    macro_rules! operators { ($($op:ident),*) => { match op { $(BinOp::$op=>BinaryOperator::$op,)* other=>BinaryOperator::Other(format!("{other:?}")) } }; }
    operators!(
        Add,
        Sub,
        Mul,
        Div,
        Rem,
        BitAnd,
        BitOr,
        BitXor,
        Shl,
        Shr,
        Eq,
        Ne,
        Lt,
        Le,
        Gt,
        Ge,
        Offset,
        AddUnchecked,
        SubUnchecked,
        MulUnchecked,
        AddWithOverflow,
        SubWithOverflow,
        MulWithOverflow
    )
}
fn unary(op: UnOp) -> UnaryOperator {
    match op {
        UnOp::Neg => UnaryOperator::Neg,
        UnOp::Not => UnaryOperator::Not,
        UnOp::PtrMetadata => UnaryOperator::PtrMetadata,
    }
}
