#![allow(dead_code, unused_variables, unused_mut)]

// Inspired by:
// - rust-lang.regex/regex-automata/src/hybrid/dfa.rs/next_state_untagged_unchecked/1414
//
// # Safety
// Calling `slice::get_unchecked` with an out-of-bounds index is undefined.

fn main() {
    let trans = [0u16, 1, 2, 3];

    let bad_offset = 9usize;
    let _bad = unsafe { *trans.get_unchecked(bad_offset) };

    let good_offset = 2usize;
    let _good = unsafe { *trans.get_unchecked(good_offset) };
}
