#![allow(dead_code, unused_variables, unused_mut)]

// Inspired by:
// - bitvecto-rs.bitvec/src/slice/api.rs/get_unchecked/479
//
// # Safety
// `index` must be in bounds of the slice.

fn main() {
    let bits = [1u8, 2, 3];

    let bad_index = 5usize;
    let _bad = unsafe { bits.get_unchecked(bad_index) };

    let good_index = 1usize;
    let _good = unsafe { bits.get_unchecked(good_index) };
}
