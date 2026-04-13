#![allow(dead_code, unused_variables, unused_mut)]

// Inspired by:
// - bitvecto-rs.bitvec/src/slice.rs/copy_within_unchecked/949
//
// # Safety
// `get_unchecked` / `get_unchecked_mut` require indexes within the slice.

fn main() {
    let mut bits = [1u8, 2, 3, 4];

    let bad_src = [1usize, 3usize];
    let bad_dest = 3usize;
    let src_len = bad_src[1] - bad_src[0];
    let value = unsafe { *bits.get_unchecked(bad_src[0]) };
    unsafe { *bits.get_unchecked_mut(bad_dest + src_len - 1) = value };

    let good_src = [0usize, 2usize];
    let good_dest = 1usize;
    let src_len = good_src[1] - good_src[0];
    let value = unsafe { *bits.get_unchecked(good_src[0]) };
    unsafe { *bits.get_unchecked_mut(good_dest + src_len - 1) = value };
}
