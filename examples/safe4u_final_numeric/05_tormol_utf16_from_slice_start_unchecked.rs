#![allow(dead_code, unused_variables, unused_mut)]

// Inspired by:
// - tormol-encode-unicode/src/utf16_char.rs/from_slice_start_unchecked/468
//
// # Safety
// Calling `slice::get_unchecked` with an out-of-bounds index is undefined.

fn main() {
    let bad_src: [u16; 1] = [0xD800];
    let _bad = unsafe {
        let first = *bad_src.get_unchecked(0);
        if (0xD800..=0xDBFF).contains(&first) {
            ([first, *bad_src.get_unchecked(1)], 2usize)
        } else {
            ([first, 0], 1usize)
        }
    };

    let good_src: [u16; 2] = [0xD800, 0xDC00];
    let _good = unsafe {
        let first = *good_src.get_unchecked(0);
        if (0xD800..=0xDBFF).contains(&first) {
            ([first, *good_src.get_unchecked(1)], 2usize)
        } else {
            ([first, 0], 1usize)
        }
    };
}
