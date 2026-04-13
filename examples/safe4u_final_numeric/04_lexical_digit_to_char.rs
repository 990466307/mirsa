#![allow(dead_code, unused_variables, unused_mut)]

// Inspired by:
// - Alexhuszagh.rust-lexical/lexical-util/src/digit.rs/digit_to_char/105
//
// # Safety
// Calling `slice::get_unchecked` with an out-of-bounds index is undefined.

fn main() {
    const TABLE: [u8; 36] = *b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ";

    let bad_digit = 40u8;
    let _bad = unsafe { *TABLE.get_unchecked(bad_digit as usize) };

    let good_digit = 15u8;
    let _good = unsafe { *TABLE.get_unchecked(good_digit as usize) };
}
