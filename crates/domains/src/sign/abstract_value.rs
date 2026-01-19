#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sign {
    Neg,
    Zero,
    Pos,
    Top,
}

pub fn join(a: Sign, b: Sign) -> Sign {
    if a == b { a } else { Sign::Top }
}

pub fn neg(x: Sign) -> Sign {
    match x {
        Sign::Neg => Sign::Pos,
        Sign::Pos => Sign::Neg,
        Sign::Zero => Sign::Zero,
        Sign::Top => Sign::Top,
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