#![allow(dead_code, unused_variables, unused_mut)]

// Inspired by:
// - petgraph.fixedbitset/src/lib.rs/copy_bit/550
//
// # Safety
// Calling `slice::get_unchecked_mut` with an out-of-bounds index is undefined.

fn main() {
    let mut good_bits = [false, true, false];
    let good_to = 1usize;
    let good_enabled = true;
    unsafe { *good_bits.get_unchecked_mut(good_to) = good_enabled };

    let mut bad_bits = [true, false, true];
    let bad_to = 7usize;
    let bad_enabled = false;
    unsafe { *bad_bits.get_unchecked_mut(bad_to) = bad_enabled };
}
