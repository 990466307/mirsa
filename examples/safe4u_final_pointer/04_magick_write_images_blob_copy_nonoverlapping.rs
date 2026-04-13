#![allow(dead_code, unused_variables, unused_mut)]

// Inspired by:
// - ImageMagick write_images_blob
//
// # Safety
// The pointers passed to `ptr::copy_nonoverlapping` must be non-null.

use std::ptr;

fn main() {
    let mut bytes = [0u8; 8];

    let bad_blob = std::ptr::null::<u8>();
    let bad_length = 4usize;
    unsafe { ptr::copy_nonoverlapping(bad_blob, bytes.as_mut_ptr(), bad_length) };

    let good_blob = b"good\0".as_ptr();
    let good_length = 4usize;
    unsafe { ptr::copy_nonoverlapping(good_blob, bytes.as_mut_ptr(), good_length) };
}
