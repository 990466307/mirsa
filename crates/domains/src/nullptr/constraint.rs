use super::NullPtr;

/// Meet two nullness constraints. `None` denotes a concrete contradiction.
pub fn constrain(current: NullPtr, wanted: NullPtr) -> Option<NullPtr> {
    match (current, wanted) {
        (NullPtr::Bot, _) | (_, NullPtr::Bot) => None,
        (NullPtr::MaybeNull, value) | (value, NullPtr::MaybeNull) => Some(value),
        (left, right) if left == right => Some(left),
        _ => None,
    }
}
