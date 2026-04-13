#![allow(dead_code, unused_variables, unused_mut)]

// Inspired by:
// - bitvecto-rs.bitvec/src/slice.rs/replace_unchecked/832
//
// # Safety
// Calling `slice::get_unchecked_mut` with an out-of-bounds index is undefined.

fn main() {
    let mut bits = [true, false, true];

    let bad_index = 7usize;
    let _bad = unsafe { std::mem::replace(bits.get_unchecked_mut(bad_index), true) };

    let good_index = 1usize;
    let _good = unsafe { std::mem::replace(bits.get_unchecked_mut(good_index), false) };
}
