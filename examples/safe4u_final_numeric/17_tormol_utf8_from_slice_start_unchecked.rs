#![allow(dead_code, unused_variables, unused_mut)]

// Inspired by:
// - tormol.encode_unicode/src/utf8_char.rs/from_slice_start_unchecked/456
//
// # Safety
// The input slice must contain enough bytes for the UTF-8 character prefix.

use std::ptr;

fn main() {
    let bad = [0xE2u8];
    unsafe {
        let first = *bad.get_unchecked(0);
        let len = 1 + if first < 0x80 { 0 } else if first < 0xE0 { 1 } else if first < 0xF0 { 2 } else { 3 };
        let mut bytes = [0u8; 4];
        ptr::copy_nonoverlapping(bad.as_ptr(), bytes.as_mut_ptr(), len);
        let _bad = (bytes, len);
    }

    let good = [0xE2u8, 0x82, 0xAC];
    unsafe {
        let first = *good.get_unchecked(0);
        let len = 1 + if first < 0x80 { 0 } else if first < 0xE0 { 1 } else if first < 0xF0 { 2 } else { 3 };
        let mut bytes = [0u8; 4];
        ptr::copy_nonoverlapping(good.as_ptr(), bytes.as_mut_ptr(), len);
        let _good = (bytes, len);
    }
}
