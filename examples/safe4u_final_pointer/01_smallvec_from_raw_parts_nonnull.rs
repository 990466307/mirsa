#![allow(dead_code, unused_variables, unused_mut)]

// Inspired by:
// - servo.rust-smallvec/src/lib.rs/from_raw_parts/1476
//
// # Safety
// The pointer passed to `NonNull::new_unchecked` must be non-null.

use std::ptr::NonNull;

fn main() {
    let bad_ptr = std::ptr::null_mut::<u8>();
    let _bad = unsafe { NonNull::new_unchecked(bad_ptr) };

    let mut value = 1u8;
    let good_ptr = &mut value as *mut u8;
    let _good = unsafe { NonNull::new_unchecked(good_ptr) };
}
