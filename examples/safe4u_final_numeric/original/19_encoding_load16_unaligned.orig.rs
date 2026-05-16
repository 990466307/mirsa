// sample_label: hsivonen.encoding_rs/src/simd_funcs.rs/load16_unaligned/18
// repo_name: hsivonen.encoding_rs
// relative_file: src/simd_funcs.rs
// function: load16_unaligned

pub unsafe fn load16_unaligned(ptr: *const u8) -> u8x16 {
    let mut simd = ::core::mem::MaybeUninit::<u8x16>::uninit();
    ::core::ptr::copy_nonoverlapping(ptr, simd.as_mut_ptr() as *mut u8, 16);
    // Safety: copied 16 bytes of initialized memory into this, it is now initialized
    simd.assume_init()
}
