#![allow(dead_code, unused_variables, unused_mut)]

// Inspired by:
// - hsivonen.encoding_rs/src/simd_funcs.rs/load16_unaligned/18
//
// # Safety
// `ptr` must be valid for reading 16 bytes.

use std::mem::MaybeUninit;
use std::ptr;

fn main() {
    let bad = [0u8; 8];
    unsafe {
        let mut simd = MaybeUninit::<[u8; 16]>::uninit();
        ptr::copy_nonoverlapping(bad.as_ptr(), simd.as_mut_ptr() as *mut u8, 16);
        let _bad = simd.assume_init();
    }

    let good = [0u8; 16];
    unsafe {
        let mut simd = MaybeUninit::<[u8; 16]>::uninit();
        ptr::copy_nonoverlapping(good.as_ptr(), simd.as_mut_ptr() as *mut u8, 16);
        let _good = simd.assume_init();
    }
}
