#![allow(dead_code, unused_variables, unused_mut)]

// Inspired by:
// - ImageMagick get_image_properties
//
// # Safety
// Each C string pointer passed to `CStr::from_ptr` must be non-null.

use std::ffi::CStr;
use std::os::raw::c_char;

fn main() {
    let bad_values: [*const c_char; 1] = [std::ptr::null()];
    let bad_num = 1usize;
    let mut bad_i = 0usize;
    while bad_i < bad_num {
        let c_value = bad_values[0];
        let _bad = unsafe { CStr::from_ptr(c_value) };
        let _ = _bad;
        bad_i += 1;
    }

    let good = b"ok\0";
    let good_values: [*const c_char; 1] = [good.as_ptr().cast::<c_char>()];
    let good_num = 1usize;
    let mut good_i = 0usize;
    while good_i < good_num {
        let c_value = good_values[0];
        let _good = unsafe { CStr::from_ptr(c_value) };
        let _ = _good;
        good_i += 1;
    }
}
