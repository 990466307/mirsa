#![allow(dead_code, unused_variables, unused_mut)]

// Inspired by:
// - chemfiles from_c
//
// # Safety
// The C string pointer passed to `CStr::from_ptr` must be non-null.

use std::ffi::CStr;
use std::os::raw::c_char;

fn main() {
    let bad_buffer = std::ptr::null::<c_char>();
    let _bad = unsafe { CStr::from_ptr(bad_buffer) };

    let good = b"ok\0";
    let good_buffer = good.as_ptr().cast::<c_char>();
    let _good = unsafe { CStr::from_ptr(good_buffer) };
}
