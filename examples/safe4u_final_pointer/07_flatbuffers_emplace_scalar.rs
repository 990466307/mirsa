#![allow(dead_code, unused_variables, unused_mut)]

// Inspired by:
// - flatbuffers emplace_scalar
//
// # Safety
// The destination pointer passed to `ptr::copy_nonoverlapping` must be non-null.

use std::mem::size_of;
use std::ptr;

fn main() {
    let bad_dst = std::ptr::null_mut::<u8>();
    let bad_x_le = 1u32.to_le();
    unsafe {
        ptr::copy_nonoverlapping(&bad_x_le as *const u32 as *const u8, bad_dst, size_of::<u32>());
    }

    let mut good_dst = [0u8; 4];
    let good_x_le = 2u32.to_le();
    unsafe {
        ptr::copy_nonoverlapping(
            &good_x_le as *const u32 as *const u8,
            good_dst.as_mut_ptr(),
            size_of::<u32>(),
        );
    }
}
