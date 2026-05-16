#![allow(dead_code, unused_variables, unused_mut)]

// Inspired by:
// - ferrilab.ferrilab/funty/src/ptr.rs/new_unchecked/1050
//
// # Safety
// `ptr` must be non-null before constructing `NonNull`.

use std::ptr::NonNull;

fn main() {
    let bad_ptr = std::ptr::null::<u8>();
    let _bad = unsafe { NonNull::new_unchecked(bad_ptr.cast_mut()) };

    let value = 1u8;
    let good_ptr = &value as *const u8;
    let _good = unsafe { NonNull::new_unchecked(good_ptr.cast_mut()) };
}
