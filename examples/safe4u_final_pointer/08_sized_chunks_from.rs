#![allow(dead_code, unused_variables, unused_mut)]

// Inspired by:
// - sized_chunks::from
//
// # Safety
// The pointers passed to `ptr::copy_nonoverlapping` must be non-null.

use std::ptr;

fn main() {
    let out_right = 4usize;
    let mut out = [0u8; 8];

    let bad_src = std::ptr::null::<u8>();
    unsafe { ptr::copy_nonoverlapping(bad_src, out.as_mut_ptr().add(0), out_right) };

    let good_src = [1u8, 2, 3, 4];
    unsafe { ptr::copy_nonoverlapping(good_src.as_ptr(), out.as_mut_ptr().add(0), out_right) };
}
