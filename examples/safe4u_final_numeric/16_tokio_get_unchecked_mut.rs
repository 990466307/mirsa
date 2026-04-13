#![allow(dead_code, unused_variables, unused_mut)]

// Inspired by:
// - tokio-rs.slab/src/lib.rs/get_unchecked_mut/848
//
// # Safety
// Calling `slice::get_unchecked_mut` with an out-of-bounds index is undefined.

fn main() {
    let mut bad_entries = [10u32, 11, 12];
    let bad_key = 9usize;
    let _bad = unsafe { bad_entries.get_unchecked_mut(bad_key) };

    let mut good_entries = [10u32, 11, 12];
    let good_key = 2usize;
    let _good = unsafe { good_entries.get_unchecked_mut(good_key) };
}
