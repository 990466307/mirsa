#![allow(dead_code, unused_variables, unused_mut)]

// Inspired by:
// - ImageMagick get_image_property
//
// # Safety
// The C string pointer passed to `CStr::from_ptr` must be non-null.

use std::ffi::CStr;
use std::os::raw::c_char;

fn main() {
    let bad_c_value = std::ptr::null::<c_char>();
    let _bad = unsafe { CStr::from_ptr(bad_c_value) };

    let good = b"ok\0";
    let good_c_value = good.as_ptr().cast::<c_char>();
    let _good = unsafe { CStr::from_ptr(good_c_value) };
}
