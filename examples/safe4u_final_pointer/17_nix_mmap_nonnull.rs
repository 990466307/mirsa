#![allow(dead_code, unused_variables, unused_mut)]

// Inspired by:
// - nix-rust/nix/src/sys/mman.rs/mmap/394
//
// # Safety
// The pointer passed to `NonNull::new_unchecked` must be non-null.

use std::ptr::NonNull;

fn main() {
    let bad_ret = std::ptr::null_mut::<u8>();
    let _bad = unsafe { NonNull::new_unchecked(bad_ret) };

    let mut good = 0u8;
    let good_ret = &mut good as *mut u8;
    let _good = unsafe { NonNull::new_unchecked(good_ret) };
}
