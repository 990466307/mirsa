use super::model::*;
use std::fmt;

impl fmt::Display for FactOutput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "抽象信息：")?;
        for fact in &self.abstract_info {
            writeln!(f, "  {}: {:?}", fact.subject, fact.value)?;
        }
        writeln!(f, "符号信息：")?;
        for fact in &self.symbolic_info {
            writeln!(f, "  {fact}")?;
        }
        Ok(())
    }
}
impl fmt::Display for SymbolicFact {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Definition { target, expression } => write!(f, "{target} = {expression}"),
            Self::Condition { expression } => write!(f, "{expression}"),
            Self::SameValue { members } => {
                write!(f, "same_value(")?;
                for (i, member) in members.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{member}")?;
                }
                write!(f, ")")
            }
            Self::PointsTo { pointer, target } => write!(f, "points_to({pointer}, {target})"),
        }
    }
}
impl fmt::Display for Expression {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Variable { name } => write!(f, "{name}"),
            Self::Allocation { identity } => write!(f, "{identity}"),
            Self::Typed { expression, .. } => write!(f, "{expression}"),
            Self::Constant { bits, ty, width } => {
                if ty == "bool" && (bits == "0" || bits == "1") {
                    return write!(f, "{}", bits == "1");
                }
                if ty.starts_with('u') {
                    return write!(f, "{bits}{ty}");
                }
                if ty.starts_with('i') {
                    if let (Ok(bits), Some(width)) = (bits.parse::<u128>(), width) {
                        if (1..=128).contains(width) {
                            return write!(
                                f,
                                "{}{ty}",
                                ((bits << (128 - width)) as i128) >> (128 - width)
                            );
                        }
                    }
                }
                write!(f, "bits::<{ty}>({bits})")
            }
            Self::Literal { token } => write!(f, "{token}"),
            Self::Binary {
                operator,
                left,
                right,
            } => write!(f, "({left} {} {right})", operator),
            Self::Unary { operator, operand } => match operator {
                UnaryOperator::Neg => write!(f, "(-{operand})"),
                UnaryOperator::Not => write!(f, "(!{operand})"),
                UnaryOperator::Deref => write!(f, "(*{operand})"),
                other => write!(f, "{other:?}({operand})"),
            },
            Self::Cast { ty, operand, .. } => write!(f, "({operand} as {ty})"),
            Self::CheckedBinary {
                operator,
                overflow,
                left,
                right,
            } => write!(
                f,
                "checked_{operator:?}_{}({left}, {right})",
                if *overflow { "overflow" } else { "value" }
            ),
            Self::Property { property, argument } => write!(f, "{}({argument})", property),
            Self::Projection { base, element } => match element {
                Projection::Deref => write!(f, "(*{base})"),
                Projection::Field(n) => write!(f, "{base}.{n}"),
                Projection::NamedField(n) => write!(f, "{base}.{n}"),
                Projection::Downcast(n) => write!(f, "variant({base}, {n})"),
                Projection::AnyIndex => write!(f, "{base}[_]"),
                Projection::ConstantIndex { offset, from_end } => {
                    write!(f, "{base}[{}{offset}]", if *from_end { "-" } else { "" })
                }
            },
            Self::Index { base, index } => write!(f, "{base}[{index}]"),
            Self::PointerOffset {
                base,
                count,
                bytes,
                subtract,
                wrapping,
            } => write!(
                f,
                "pointer_offset({base}, {count}, bytes={bytes}, subtract={subtract}, wrapping={wrapping})"
            ),
            Self::PointerFromAddress { address } => write!(f, "pointer_from_address({address})"),
            Self::Call { callee, arguments } => {
                write!(f, "{callee}(")?;
                for (i, a) in arguments.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{a}")?;
                }
                write!(f, ")")
            }
            Self::TypeLayout { ty, alignment } => write!(
                f,
                "{}::<{ty}>()",
                if *alignment { "align_of" } else { "size_of" }
            ),
            Self::OpaqueValue { identity, .. } => write!(f, "{identity}"),
        }
    }
}
impl fmt::Display for BinaryOperator {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = match self {
            Self::Add => "+",
            Self::Sub => "-",
            Self::Mul => "*",
            Self::Div => "/",
            Self::Rem => "%",
            Self::BitAnd => "&",
            Self::BitOr => "|",
            Self::BitXor => "^",
            Self::Shl => "<<",
            Self::Shr => ">>",
            Self::Eq => "==",
            Self::Ne => "!=",
            Self::Lt => "<",
            Self::Le => "<=",
            Self::Gt => ">",
            Self::Ge => ">=",
            Self::Implies => "=>",
            Self::LogicalAnd => "&&",
            Self::LogicalOr => "||",
            other => return write!(f, "{other:?}"),
        };
        write!(f, "{text}")
    }
}
impl fmt::Display for Property {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = match self {
            Self::Length => "len",
            Self::IsNan => "is_nan",
            Self::IsInfinite => "is_infinite",
            Self::IsNull => "is_null",
            Self::IsLive => "is_live",
            Self::IsFreed => "is_freed",
            Self::Base => "base",
            Self::Address => "address",
            Self::Offset => "offset",
            Self::Extent => "extent",
            Self::LayoutSize => "layout_size",
            Self::LayoutAlign => "layout_align",
            Self::Discriminant => "discriminant",
            Self::Math => "math",
            Self::Bits => "bits",
            Self::Variant => "variant",
            Self::Buffer => "buffer",
            Self::MayHaveAddressTaken => "may_have_address_taken",
            Self::MayEscape => "may_escape",
            Self::Allocation => "allocation",
        };
        write!(f, "{text}")
    }
}
