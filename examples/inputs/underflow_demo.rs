pub fn empty_slice_underflow(data: &[u8], index: usize) -> Option<u8> {
    if index <= data.len() - 1 {
        Some(unsafe { *data.get_unchecked(index) })
    } else {
        None
    }
}
