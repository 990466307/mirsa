#![allow(dead_code, unused_variables, unused_mut)]

// Inspired by:
// - bitvecto-rs.bitvec/src/slice.rs/split_at_unchecked_mut/902
//
// # Safety
// `slice::split_at_mut_unchecked` requires the index to be within the slice.

fn main() {
    let mut bits = [1u8, 2, 3];

    let bad_mid = 9usize;
    let _bad = unsafe { bits.split_at_mut_unchecked(bad_mid) };

    let good_mid = 1usize;
    let (_left, _right) = unsafe { bits.split_at_mut_unchecked(good_mid) };
}
