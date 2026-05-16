// sample_label: isahc body buffer conversion
// note: reconstructed from isahc-style owned buffer conversion; this item is not present in local Safe4U raw_code.

fn into_vec(mut slice: Box<[u8]>, len: usize) -> Vec<u8> {
    assert!(len <= slice.len());
    let ptr = slice.as_mut_ptr();
    let cap = slice.len();
    std::mem::forget(slice);
    unsafe { Vec::from_raw_parts(ptr, len, cap) }
}
