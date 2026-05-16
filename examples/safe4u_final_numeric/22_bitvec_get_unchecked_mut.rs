#![allow(dead_code, unused_variables, unused_mut)]

// Inspired by:
// - bitvecto-rs.bitvec/src/slice/api.rs/get_unchecked_mut/521
//
// # Safety
// `index` must be in bounds of the mutable slice.

fn main() {
    let mut bits = [1u8, 2, 3];

    let bad_index = 3usize;
    let _bad = unsafe { bits.get_unchecked_mut(bad_index) };

    let good_index = 1usize;
    let _good = unsafe { bits.get_unchecked_mut(good_index) };
}
