#![allow(dead_code, unused_variables, unused_mut)]

// Inspired by:
// - ferrilab.ferrilab/funty/src/ptr.rs/make_slice/858
//
// # Safety
// `ptr` must be non-null and valid for `len` elements.

fn main() {
    let bad_ptr = std::ptr::null::<u8>();
    let _bad = unsafe { std::slice::from_raw_parts(bad_ptr, 4) };

    let good = [1u8, 2, 3, 4];
    let _good = unsafe { std::slice::from_raw_parts(good.as_ptr(), good.len()) };
}
