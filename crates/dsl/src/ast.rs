use std::ops::Range;

pub type Span = Range<usize>;

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct Diagnostic {
    pub span: Span,
    pub message: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RequestKind {
    Check,
    Facts,
}

#[derive(Clone, Debug)]
pub struct Document {
    pub(crate) source: String,
    pub(crate) statement: Statement,
}

impl Document {
    pub fn source(&self) -> &str {
        &self.source
    }
    pub fn statement(&self) -> &Statement {
        &self.statement
    }
}

#[derive(Clone, Debug)]
pub struct Statement {
    pub kind: RequestKind,
    pub location: String,
    pub expression: Expr,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub struct Expr {
    pub kind: ExprKind,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub enum ExprKind {
    Binding(String),
    Number(String),
    Bool(bool),
    Unary(Unary, Box<Expr>),
    Binary(Binary, Box<Expr>, Box<Expr>),
    Cast(Box<Expr>, String),
    Field(Box<Expr>, String),
    Index(Box<Expr>, Box<Expr>),
    Builtin(Builtin, Vec<Expr>),
    TypeLayout { align: bool, ty: TypeRef },
}

#[derive(Clone, Debug)]
pub struct TypeRef {
    pub name: String,
    pub binding: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unary {
    Not,
    Neg,
    Deref,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Binary {
    Implies,
    Or,
    And,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    BitOr,
    BitXor,
    BitAnd,
    Shl,
    Shr,
    Add,
    Sub,
    Mul,
    Div,
    Rem,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Builtin {
    Math,
    Len,
    IsNan,
    IsInfinite,
    IsNull,
    IsLive,
    IsFreed,
    SameAllocation,
    Base,
    Address,
    Offset,
    Extent,
    LayoutSize,
    LayoutAlign,
    Discriminant,
}

impl Builtin {
    pub fn named(name: &str) -> Option<Self> {
        Some(match name {
            "math" => Self::Math,
            "len" => Self::Len,
            "is_nan" => Self::IsNan,
            "is_infinite" => Self::IsInfinite,
            "is_null" => Self::IsNull,
            "is_live" => Self::IsLive,
            "is_freed" => Self::IsFreed,
            "same_allocation" => Self::SameAllocation,
            "base" => Self::Base,
            "address" => Self::Address,
            "offset" => Self::Offset,
            "extent" => Self::Extent,
            "layout_size" => Self::LayoutSize,
            "layout_align" => Self::LayoutAlign,
            "discriminant" => Self::Discriminant,
            _ => return None,
        })
    }

    pub fn arity(self) -> usize {
        if self == Self::SameAllocation { 2 } else { 1 }
    }
}

pub fn is_scalar(name: &str) -> bool {
    matches!(
        name,
        "bool"
            | "char"
            | "f32"
            | "f64"
            | "i8"
            | "i16"
            | "i32"
            | "i64"
            | "i128"
            | "isize"
            | "u8"
            | "u16"
            | "u32"
            | "u64"
            | "u128"
            | "usize"
    )
}

pub fn is_name(name: &str) -> bool {
    let mut chars = name.bytes();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == b'_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == b'_')
}
