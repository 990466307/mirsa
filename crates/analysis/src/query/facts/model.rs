use serde::Serialize;

/// Owned query output, independent of the compiler context.
#[derive(Clone, Debug, Serialize)]
pub struct FactOutput {
    pub abstract_info: Vec<AbstractFact>,
    pub symbolic_info: Vec<SymbolicFact>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct AbstractFact {
    pub subject: Expression,
    pub value: super::super::FactValue,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Expression {
    Variable {
        name: String,
    },
    Allocation {
        identity: String,
    },
    Typed {
        ty: String,
        expression: Box<Self>,
    },
    Constant {
        bits: String,
        width: Option<u64>,
        ty: String,
    },
    Literal {
        token: String,
    },
    Binary {
        operator: BinaryOperator,
        left: Box<Self>,
        right: Box<Self>,
    },
    Unary {
        operator: UnaryOperator,
        operand: Box<Self>,
    },
    Cast {
        cast_kind: String,
        ty: String,
        operand: Box<Self>,
    },
    CheckedBinary {
        operator: BinaryOperator,
        overflow: bool,
        left: Box<Self>,
        right: Box<Self>,
    },
    Property {
        property: Property,
        argument: Box<Self>,
    },
    Projection {
        base: Box<Self>,
        element: Projection,
    },
    Index {
        base: Box<Self>,
        index: Box<Self>,
    },
    PointerOffset {
        base: Box<Self>,
        count: Box<Self>,
        bytes: u64,
        subtract: bool,
        wrapping: bool,
    },
    PointerFromAddress {
        address: Box<Self>,
    },
    Call {
        callee: String,
        arguments: Vec<Self>,
    },
    TypeLayout {
        ty: String,
        alignment: bool,
    },
    /// Constant or place identity outside scalar/access-path syntax; never an encoded formula.
    OpaqueValue {
        identity: String,
        ty: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize)]
pub enum Projection {
    Deref,
    Field(usize),
    Downcast(usize),
    AnyIndex,
    ConstantIndex { offset: u64, from_end: bool },
    NamedField(String),
}
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize)]
pub enum BinaryOperator {
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
    Cmp,
    AddUnchecked,
    SubUnchecked,
    MulUnchecked,
    ShlUnchecked,
    ShrUnchecked,
    AddWithOverflow,
    SubWithOverflow,
    MulWithOverflow,
    Implies,
    LogicalAnd,
    LogicalOr,
    Other(String),
}
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize)]
pub enum UnaryOperator {
    Neg,
    Not,
    Deref,
    PtrMetadata,
    Other(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
pub enum Property {
    Length,
    IsNan,
    IsInfinite,
    IsNull,
    IsLive,
    IsFreed,
    Base,
    Address,
    Offset,
    Extent,
    LayoutSize,
    LayoutAlign,
    Discriminant,
    Math,
    Bits,
    Variant,
    Buffer,
    MayHaveAddressTaken,
    MayEscape,
    Allocation,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SymbolicFact {
    Definition {
        target: Expression,
        expression: Expression,
    },
    /// Stored-value equivalence, not a Rust `==` comparison (notably for NaN).
    SameValue {
        members: Vec<Expression>,
    },
    Condition {
        expression: Expression,
    },
    PointsTo {
        pointer: Expression,
        target: Expression,
    },
}

impl Expression {
    pub fn property(self, property: Property) -> Self {
        Self::Property {
            property,
            argument: Box::new(self),
        }
    }
    pub fn variable(name: impl Into<String>) -> Self {
        Self::Variable { name: name.into() }
    }
}
