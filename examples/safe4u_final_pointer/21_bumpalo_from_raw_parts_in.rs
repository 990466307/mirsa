#![allow(dead_code, unused_variables, unused_mut)]

// Inspired by:
// - fitzgen.bumpalo/src/collections/string.rs/from_raw_parts_in/763
//
// # Safety
// `buf` must be non-null and valid for `length` initialized bytes within `capacity`.

fn main() {
    let bad_buf = std::ptr::null_mut::<u8>();
    unsafe {
        let vec = Vec::from_raw_parts(bad_buf, 1usize, 4usize);
        let _bad = String::from_utf8_unchecked(vec);
    }

    let mut good = Vec::from(*b"ok");
    let ptr = good.as_mut_ptr();
    let len = good.len();
    let cap = good.capacity();
    std::mem::forget(good);
    unsafe {
        let vec = Vec::from_raw_parts(ptr, len, cap);
        let _good = String::from_utf8_unchecked(vec);
    }
}
