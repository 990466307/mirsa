#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sign {
    Neg,
    Zero,
    Pos,
    Top,
    Bot,
}

pub fn join(a: Sign, b: Sign) -> Sign {
    use Sign::*;
    match (a, b) {
        (Bot, x) | (x, Bot) => x,
        (x, y) if x == y => x,
        _ => Top,
    }
}

pub fn meet(a: Sign, b: Sign) -> Sign {
    use Sign::*;
    match (a, b) {
        (Bot, _) | (_, Bot) => Bot,
        (Top, x) | (x, Top) => x,
        (x, y) if x == y => x,
        _ => Bot,
    }
}

pub fn neg(x: Sign) -> Sign {
    match x {
        Sign::Neg => Sign::Pos,
        Sign::Pos => Sign::Neg,
        Sign::Zero => Sign::Zero,
        _ => Sign::Top,
    }
}

pub fn add(a: Sign, b: Sign) -> Sign {
    use Sign::*;
    match (a, b) {
        (Zero, x) | (x, Zero) => x,
        (Pos, Pos) => Pos,
        (Neg, Neg) => Neg,
        _ => Top,
    }
}

pub fn sub(a: Sign, b: Sign) -> Sign {
    add(a, neg(b))
}

pub fn mul(a: Sign, b: Sign) -> Sign {
    use Sign::*;
    match (a, b) {
        (Zero, _) | (_, Zero) => Zero,
        (Pos, Pos) => Pos,
        (Neg, Neg) => Pos,
        (Pos, Neg) | (Neg, Pos) => Neg,
        _ => Top,
    }
}

pub fn div(a: Sign, b: Sign) -> Sign {
    use Sign::*;
    match (a, b) {
        (Zero, _) => Zero,
        (Pos, Pos) => Pos,
        (Pos, Neg) => Neg,
        (Neg, Pos) => Neg,
        (Neg, Neg) => Pos,
        _ => Top,
    }
}

fn bool_not(s: Sign) -> Sign {
    match s {
        Sign::Pos => Sign::Zero,
        Sign::Zero => Sign::Pos,
        Sign::Neg => Sign::Top,
        Sign::Top => Sign::Top,
        Sign::Bot => Sign::Bot,
    }
}

// Boolean result encoding:
// Pos=true, Zero=false, Top=unknown, Bot=unreachable.
pub fn lt(a: Sign, b: Sign) -> Sign {
    use Sign::*;
    match (a, b) {
        (Bot, _) | (_, Bot) => Bot,
        (Neg, Zero) | (Neg, Pos) | (Zero, Pos) => Pos,
        (Pos, Zero) | (Pos, Neg) | (Zero, Zero) | (Zero, Neg) => Zero,
        (Neg, Neg)
        | (Pos, Pos)
        | (Top, _)
        | (_, Top) => Top,
    }
}

pub fn eq(a: Sign, b: Sign) -> Sign {
    use Sign::*;
    match (a, b) {
        (Bot, _) | (_, Bot) => Bot,
        (Zero, Zero) => Pos,
        (Neg, Zero) | (Zero, Neg) | (Pos, Zero) | (Zero, Pos) | (Neg, Pos) | (Pos, Neg) => Zero,
        (Neg, Neg)
        | (Pos, Pos)
        | (Top, _)
        | (_, Top) => Top,
    }
}

pub fn gt(a: Sign, b: Sign) -> Sign {
    lt(b, a)
}

pub fn le(a: Sign, b: Sign) -> Sign {
    bool_not(gt(a, b))
}

pub fn ge(a: Sign, b: Sign) -> Sign {
    bool_not(lt(a, b))
}

pub fn neq(a: Sign, b: Sign) -> Sign {
    bool_not(eq(a, b))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lt_pos_pos_is_unknown_not_false() {
        assert_eq!(lt(Sign::Pos, Sign::Pos), Sign::Top);
    }

    #[test]
    fn eq_zero_zero_true_and_eq_pos_pos_unknown() {
        assert_eq!(eq(Sign::Zero, Sign::Zero), Sign::Pos);
        assert_eq!(eq(Sign::Pos, Sign::Pos), Sign::Top);
    }
}
