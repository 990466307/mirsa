#![allow(dead_code, unused_variables, unused_mut)]

// Inspired by:
// - isahc buffer from_raw_parts
//
// # Safety
// The pointer passed to `Vec::from_raw_parts` must be non-null.

fn main() {
    let bad_ptr = std::ptr::null_mut::<u8>();
    let bad_len = 2usize;
    let bad_capacity = 2usize;
    let _bad = unsafe { Vec::from_raw_parts(bad_ptr, bad_len, bad_capacity) };

    let mut good_buf = [0u8; 4];
    let good_ptr = good_buf.as_mut_ptr();
    let good_len = 2usize;
    let good_capacity = good_buf.len();
    let _good = unsafe { Vec::from_raw_parts(good_ptr, good_len, good_capacity) };
}
