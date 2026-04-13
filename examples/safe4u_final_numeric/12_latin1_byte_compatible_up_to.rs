#![allow(dead_code, unused_variables, unused_mut)]

// Inspired by:
// - encoding_rs/src/single_byte.rs/latin1_byte_compatible_up_to/254
//
// # Safety
// Calling `slice::get_unchecked` with an out-of-bounds index is undefined.

fn main() {
    let table = [0u8; 128];

    // bad loop
    let mut bad_total = 0usize;
    let bad_bytes = [0xC1u8, 0x00];
    loop {
        let non_ascii = if bad_total == 0 { bad_bytes[bad_total] as usize } else { 0x100 };
        let _ = unsafe { *(table.get_unchecked(non_ascii - 0x80usize)) };
        bad_total += 1;
        if bad_total >= 2 {
            break;
        }
    }

    // good loop
    let mut good_total = 0usize;
    let good_bytes = [0xC1u8, 0xC2];
    loop {
        let non_ascii = good_bytes[good_total] as usize;
        let _ = unsafe { *(table.get_unchecked(non_ascii - 0x80usize)) };
        good_total += 1;
        if good_total >= 2 {
            break;
        }
    }
}
