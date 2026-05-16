#![allow(dead_code, unused_variables, unused_mut)]

// Inspired by:
// - Alexhuszagh.rust-lexical/lexical-util/src/algorithm.rs/copy_to_dst/12
//
// # Safety
// `dst` must be valid for `src.len()` writes.

use std::ptr;

fn main() {
    let src = [1u8, 2, 3, 4];

    let mut bad_dst = [0u8; 2];
    unsafe {
        ptr::copy_nonoverlapping(src.as_ptr(), bad_dst.as_mut_ptr(), src.len());
        let _bad = src.len();
    }

    let mut good_dst = [0u8; 4];
    unsafe {
        ptr::copy_nonoverlapping(src.as_ptr(), good_dst.as_mut_ptr(), src.len());
        let _good = src.len();
    }
}
