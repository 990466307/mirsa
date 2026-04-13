#![allow(dead_code, unused_variables, unused_mut)]

// Inspired by:
// - Alexhuszagh.rust-lexical/lexical-parse-float/src/bigint.rs/get_unchecked/718
//
// # Safety
// Calling `slice::get_unchecked` with an out-of-bounds index is undefined
// behavior even if the resulting reference is not used.

fn main() {
    let inner = [11u32, 22, 33];

    let bad_index = 4usize;
    let _bad = unsafe {
        let len = inner.len();
        inner.get_unchecked(len - bad_index - 1)
    };

    let good_index = 1usize;
    let _good = unsafe {
        let len = inner.len();
        inner.get_unchecked(len - good_index - 1)
    };
}
