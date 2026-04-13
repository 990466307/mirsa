#![allow(dead_code, unused_variables, unused_mut)]

// Inspired by:
// - nix-rust/nix/src/sys/mman.rs/mremap/458
//
// # Safety
// The pointer passed to `NonNull::new_unchecked` must be non-null.

use std::ptr::NonNull;

fn main() {
    let mut backing = 0u8;
    let addr = unsafe { NonNull::new_unchecked((&mut backing as *mut u8).cast::<std::ffi::c_void>()) };
    let _ = addr;

    let bad_ret = std::ptr::null_mut::<std::ffi::c_void>();
    let _bad = unsafe { NonNull::new_unchecked(bad_ret) };

    let mut good_val = 1u8;
    let good_ret = (&mut good_val as *mut u8).cast::<std::ffi::c_void>();
    let _good = unsafe { NonNull::new_unchecked(good_ret) };
}
