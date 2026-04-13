#![allow(dead_code, unused_variables, unused_mut)]

// Inspired by:
// - flatbuffers read_scalar
//
// # Safety
// The source pointer passed to `ptr::copy_nonoverlapping` must be non-null.

use std::mem::size_of;
use std::ptr;

fn main() {
    let mut mem = [0u8; 4];

    let bad_src = std::ptr::null::<u8>();
    unsafe { ptr::copy_nonoverlapping(bad_src, mem.as_mut_ptr(), size_of::<u32>()) };

    let good_src = [1u8, 2, 3, 4];
    unsafe { ptr::copy_nonoverlapping(good_src.as_ptr(), mem.as_mut_ptr(), size_of::<u32>()) };
}
