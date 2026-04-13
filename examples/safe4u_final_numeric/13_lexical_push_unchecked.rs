#![allow(dead_code, unused_variables, unused_mut)]

// Inspired by:
// - Alexhuszagh.rust-lexical/lexical-parse-float/src/bigint.rs/push_unchecked/360
//
// # Safety
// Calling `slice::get_unchecked_mut` with an out-of-bounds index is undefined.

use std::mem::MaybeUninit;

fn main() {
    let mut bad_data: [MaybeUninit<u32>; 2] = [MaybeUninit::uninit(), MaybeUninit::uninit()];
    let bad_len = 2usize;
    unsafe { *bad_data.get_unchecked_mut(bad_len) = MaybeUninit::new(7u32) };

    let mut good_data: [MaybeUninit<u32>; 2] = [MaybeUninit::uninit(), MaybeUninit::uninit()];
    let good_len = 1usize;
    unsafe { *good_data.get_unchecked_mut(good_len) = MaybeUninit::new(7u32) };
}
