use super::abstract_value::{Interval, intersect};
use rustc_middle::mir::BinOp;

pub(super) fn arithmetic(
    op: BinOp,
    left: Interval,
    right: Interval,
    bits: u64,
    signed: bool,
) -> (Interval, Interval) {
    let bounds = if signed {
        if bits == 128 {
            Interval::top()
        } else {
            Interval::new(-(1i128 << (bits - 1)), (1i128 << (bits - 1)) - 1)
        }
    } else if bits == 128 {
        Interval::new(0, i128::MAX)
    } else {
        Interval::new(0, (1i128 << bits) - 1)
    };
    if left.is_empty() || right.is_empty() {
        return (Interval::empty(), Interval::empty());
    }
    if !signed && bits == 128 && (left.high == i128::MAX || right.high == i128::MAX) {
        return (Interval::top(), Interval::new(0, 1));
    }
    let left = intersect(&left, &bounds);
    let right = intersect(&right, &bounds);
    if left.is_empty() || right.is_empty() {
        return (Interval::empty(), Interval::empty());
    }
    if left.low == left.high && right.low == right.high {
        let (raw, overflow) = if signed {
            let (raw, overflow) = match op {
                BinOp::Add | BinOp::AddWithOverflow => left.low.overflowing_add(right.low),
                BinOp::Sub | BinOp::SubWithOverflow => left.low.overflowing_sub(right.low),
                BinOp::Mul | BinOp::MulWithOverflow => left.low.overflowing_mul(right.low),
                _ => unreachable!(),
            };
            (
                raw as u128,
                overflow || raw < bounds.low || raw > bounds.high,
            )
        } else {
            let a = left.low as u128;
            let b = right.low as u128;
            let (raw, overflow) = match op {
                BinOp::Add | BinOp::AddWithOverflow => a.overflowing_add(b),
                BinOp::Sub | BinOp::SubWithOverflow => a.overflowing_sub(b),
                BinOp::Mul | BinOp::MulWithOverflow => a.overflowing_mul(b),
                _ => unreachable!(),
            };
            (raw, overflow || (bits < 128 && raw > bounds.high as u128))
        };
        let raw = if bits < 128 {
            raw & ((1u128 << bits) - 1)
        } else {
            raw
        };
        let result = if signed && bits < 128 && raw & (1u128 << (bits - 1)) != 0 {
            (raw | (u128::MAX << bits)) as i128
        } else if !signed && raw > i128::MAX as u128 {
            return (
                Interval::top(),
                Interval::new(overflow as i128, overflow as i128),
            );
        } else {
            raw as i128
        };
        return (
            Interval::new(result, result),
            Interval::new(overflow as i128, overflow as i128),
        );
    }
    let mut low = i128::MAX;
    let mut high = i128::MIN;
    for a in [left.low, left.high] {
        for b in [right.low, right.high] {
            let value = match op {
                BinOp::Add | BinOp::AddWithOverflow => a.checked_add(b),
                BinOp::Sub | BinOp::SubWithOverflow => a.checked_sub(b),
                BinOp::Mul | BinOp::MulWithOverflow => a.checked_mul(b),
                _ => unreachable!(),
            };
            let Some(value) = value else {
                return (
                    if !signed && bits == 128 {
                        Interval::top()
                    } else {
                        bounds
                    },
                    Interval::new(0, 1),
                );
            };
            low = low.min(value);
            high = high.max(value);
        }
    }
    if !signed && bits == 128 && high > bounds.high {
        return (Interval::top(), Interval::new(0, 1));
    }
    if low >= bounds.low && high <= bounds.high {
        (Interval::new(low, high), Interval::new(0, 0))
    } else {
        let always = high < bounds.low || low > bounds.high;
        (
            if !signed && bits == 128 {
                Interval::top()
            } else {
                bounds
            },
            Interval::new(always as i128, 1),
        )
    }
}
