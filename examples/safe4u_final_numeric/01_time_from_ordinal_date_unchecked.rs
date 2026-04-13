#![allow(dead_code, unused_variables, unused_mut)]

// Inspired by:
// - time-rs.time/time/src/date.rs/__from_ordinal_date_unchecked/84
//
// # Safety
// The argument to `NonZero::new_unchecked` must not be zero.

use std::num::NonZeroI32;

fn main() {
    let bad_ordinal = 0u16;
    let _bad = unsafe { NonZeroI32::new_unchecked(bad_ordinal as i32) };

    let good_ordinal = 1u16;
    let _good = unsafe { NonZeroI32::new_unchecked(good_ordinal as i32) };
}
