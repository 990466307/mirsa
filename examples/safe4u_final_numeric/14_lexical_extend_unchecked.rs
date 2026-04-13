#![allow(dead_code, unused_variables, unused_mut)]

// Inspired by:
// - Alexhuszagh.rust-lexical/lexical-parse-float/src/bigint.rs/extend_unchecked/414
//
// # Safety
// `slice::get_unchecked_mut` writes. The safety property is still
// `new_len <= capacity`.

fn main() {
    let mut bad_data = [0u32; 2];
    let bad_len = 2usize;
    let extra = [1u32, 2u32];
    let new_len = bad_len + extra.len();
    unsafe { *bad_data.get_unchecked_mut(new_len - 1) = extra[1] };

    let mut good_data = [0u32; 3];
    let good_len = 1usize;
    let extra = [1u32, 2u32];
    let new_len = good_len + extra.len();
    unsafe { *good_data.get_unchecked_mut(new_len - 1) = extra[1] };
}
