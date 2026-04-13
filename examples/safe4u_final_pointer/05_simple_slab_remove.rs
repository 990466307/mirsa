#![allow(dead_code, unused_variables, unused_mut)]

// Inspired by:
// - tokio slab remove path using raw pointers
//
// # Safety
// The raw pointers used by `ptr::read` and `ptr::write` must be non-null.

use std::ptr;

fn main() {
    let mut elem = 1u32;
    let mut last_elem = 2u32;

    let bad_ptr = std::ptr::null_mut::<u32>();
    unsafe {
        elem = ptr::read(bad_ptr);
        last_elem = ptr::read(bad_ptr);
        ptr::write(bad_ptr, last_elem);
    }

    let good_ptr = &mut elem as *mut u32;
    let good_last_ptr = &mut last_elem as *mut u32;
    unsafe {
        elem = ptr::read(good_ptr);
        last_elem = ptr::read(good_last_ptr);
        ptr::write(good_ptr, last_elem);
    }
}
