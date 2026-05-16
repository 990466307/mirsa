#![allow(dead_code, unused_variables, unused_mut)]

// Inspired by:
// - hsivonen.encoding_rs/src/simd_funcs.rs/store16_unaligned/34
//
// # Safety
// `ptr` must be valid for writing 16 bytes.

use std::ptr;

fn main() {
    let simd = [1u8; 16];

    let mut bad = [0u8; 8];
    unsafe {
        ptr::copy_nonoverlapping(&simd as *const [u8; 16] as *const u8, bad.as_mut_ptr(), 16);
    }

    let mut good = [0u8; 16];
    unsafe {
        ptr::copy_nonoverlapping(&simd as *const [u8; 16] as *const u8, good.as_mut_ptr(), 16);
    }
}
