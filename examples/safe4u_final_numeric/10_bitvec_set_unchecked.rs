#![allow(dead_code, unused_variables, unused_mut)]

// Inspired by:
// - bitvecto-rs.bitvec/src/slice.rs/set_unchecked/786
//
// # Safety
// Calling `slice::get_unchecked_mut` with an out-of-bounds index is undefined.

fn main() {
    let mut bits = [false, false, true];

    let bad_index = 9usize;
    unsafe { *bits.get_unchecked_mut(bad_index) = true };

    let good_index = 0usize;
    unsafe { *bits.get_unchecked_mut(good_index) = true };
}
