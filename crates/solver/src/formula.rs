#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ScalarType {
    pub bits: u32,
    pub signed: bool,
}

impl ScalarType {
    pub const BOOL: Self = Self {
        bits: 1,
        signed: false,
    };
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BinaryOp {
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    And,
    Or,
    Xor,
    Shl,
    Shr,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnaryOp {
    Neg,
    Not,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Comparison {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

#[derive(Clone, Debug)]
pub enum Term {
    Variable(u32, ScalarType),
    Constant(u128, ScalarType),
    Binary(BinaryOp, Box<Term>, Box<Term>),
    Unary(UnaryOp, Box<Term>),
    Cast(Box<Term>, ScalarType),
    Boolean(Box<Formula>),
    Overflow(BinaryOp, Box<Term>, Box<Term>),
    FromInteger(Box<IntTerm>, ScalarType),
    FloatBits(Box<FloatTerm>),
    FloatToInteger(Box<FloatTerm>, ScalarType),
    Select(u32, Box<Term>, ScalarType),
}

impl Term {
    pub fn scalar_type(&self) -> Option<ScalarType> {
        let ty = match self {
            Self::Variable(_, ty) | Self::Constant(_, ty) => *ty,
            Self::Binary(op, a, b) => {
                let a = a.scalar_type()?;
                let b = b.scalar_type()?;
                if !matches!(op, BinaryOp::Shl | BinaryOp::Shr) && a != b {
                    return None;
                }
                a
            }
            Self::Unary(_, a) => a.scalar_type()?,
            Self::Cast(a, ty) => {
                a.scalar_type()?;
                *ty
            }
            Self::Boolean(_) => ScalarType::BOOL,
            Self::FromInteger(_, ty) | Self::FloatToInteger(_, ty) | Self::Select(_, _, ty) => *ty,
            Self::FloatBits(value) => ScalarType {
                bits: value.bits()?,
                signed: false,
            },
            Self::Overflow(op, a, b) => {
                if !matches!(op, BinaryOp::Add | BinaryOp::Sub | BinaryOp::Mul)
                    || a.scalar_type()? != b.scalar_type()?
                {
                    return None;
                }
                ScalarType::BOOL
            }
        };
        matches!(ty.bits, 1 | 8 | 16 | 32 | 64 | 128).then_some(ty)
    }
}

#[derive(Clone, Debug)]
pub enum IntTerm {
    Variable(u32),
    Constant(String),
    FromBits(Term),
    Binary(BinaryOp, Box<IntTerm>, Box<IntTerm>),
    Neg(Box<IntTerm>),
    Bitwise(BinaryOp, Box<IntTerm>, Box<IntTerm>, u32),
}

#[derive(Clone, Debug)]
pub enum FloatTerm {
    FromBits(Term),
    FromInteger(Term, u32),
    FromMath(IntTerm, u32),
    Cast(Box<FloatTerm>, u32),
    Binary(BinaryOp, Box<FloatTerm>, Box<FloatTerm>),
    Neg(Box<FloatTerm>),
}

impl FloatTerm {
    pub fn bits(&self) -> Option<u32> {
        let bits = match self {
            Self::FromBits(t) => t.scalar_type()?.bits,
            Self::FromInteger(_, bits) | Self::FromMath(_, bits) | Self::Cast(_, bits) => *bits,
            Self::Binary(op, a, b) => {
                if a.bits()? != b.bits()?
                    || !matches!(
                        op,
                        BinaryOp::Add
                            | BinaryOp::Sub
                            | BinaryOp::Mul
                            | BinaryOp::Div
                            | BinaryOp::Rem
                    )
                {
                    return None;
                }
                a.bits()?
            }
            Self::Neg(a) => a.bits()?,
        };
        matches!(bits, 32 | 64).then_some(bits)
    }
}

#[derive(Clone, Debug)]
pub enum Formula {
    Literal(bool),
    MathCompare(Comparison, IntTerm, IntTerm),
    Compare(Comparison, Term, Term),
    FloatCompare(Comparison, FloatTerm, FloatTerm),
    FloatSame(FloatTerm, FloatTerm),
    IsNan(FloatTerm),
    IsInfinite(FloatTerm),
    Not(Box<Formula>),
    All(Vec<Formula>),
    Any(Vec<Formula>),
}
