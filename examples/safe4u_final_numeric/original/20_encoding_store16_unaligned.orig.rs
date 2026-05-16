// sample_label: hsivonen.encoding_rs/src/simd_funcs.rs/store16_unaligned/34
// repo_name: hsivonen.encoding_rs
// relative_file: src/simd_funcs.rs
// function: store16_unaligned

pub unsafe fn store16_unaligned(ptr: *mut u8, s: u8x16) {
    ::core::ptr::copy_nonoverlapping(&s as *const u8x16 as *const u8, ptr, 16);
}
