#![allow(dead_code, unused_variables, unused_mut)]

// Inspired by:
// - tokio-rs.slab/src/lib.rs/get_unchecked/816
//
// # Safety
// Calling `slice::get_unchecked` with an out-of-bounds index is undefined.

fn main() {
    let bad_entries = [10u32, 11, 12];
    let bad_key = 7usize;
    let _bad = unsafe { bad_entries.get_unchecked(bad_key) };

    let good_entries = [10u32, 11, 12];
    let good_key = 1usize;
    let _good = unsafe { good_entries.get_unchecked(good_key) };
}
