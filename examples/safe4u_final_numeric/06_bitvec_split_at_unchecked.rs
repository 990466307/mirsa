#![allow(dead_code, unused_variables, unused_mut)]

// Inspired by:
// - bitvecto-rs.bitvec/src/slice.rs/split_at_unchecked/876
//
// # Safety
// `slice::split_at_unchecked` requires the index to be within the slice.

fn main() {
    let bits = [1u8, 2, 3];

    let bad_mid = 5usize;
    let _bad = unsafe { bits.split_at_unchecked(bad_mid) };

    let good_mid = 2usize;
    let _good = unsafe { bits.split_at_unchecked(good_mid) };
}
