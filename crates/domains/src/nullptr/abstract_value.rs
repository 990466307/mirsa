#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NullPtr {
    Bot,
    Null,
    NonNull,
    MaybeNull,
}

impl std::fmt::Display for NullPtr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            NullPtr::Bot => write!(f, "Bot"),
            NullPtr::Null => write!(f, "Null"),
            NullPtr::NonNull => write!(f, "NonNull"),
            NullPtr::MaybeNull => write!(f, "MaybeNull"),
        }
    }
}

pub fn join(a: NullPtr, b: NullPtr) -> NullPtr {
    use NullPtr::*;
    match (a, b) {
        (Bot, x) | (x, Bot) => x,
        (x, y) if x == y => x,
        _ => MaybeNull,
    }
}
