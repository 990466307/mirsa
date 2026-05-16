// sample_label: Alexhuszagh.rust-lexical/lexical-util/src/algorithm.rs/copy_to_dst/12
// repo_name: Alexhuszagh.rust-lexical
// relative_file: lexical-util/src/algorithm.rs
// function: copy_to_dst

pub unsafe fn copy_to_dst<Bytes: AsRef<[u8]>>(dst: &mut [u8], src: Bytes) -> usize {
    debug_assert!(dst.len() >= src.as_ref().len());

    // SAFETY: safe, if `dst.len() <= src.len()`.
    let src = src.as_ref();
    unsafe { ptr::copy_nonoverlapping(src.as_ptr(), dst.as_mut_ptr(), src.len()) };

    src.len()
}
