pub fn safe_access(data: &[u8], index: usize) -> u8 {
    if index < data.len() {
        unsafe { *data.get_unchecked(index) }
    } else {
        0
    }
}

pub fn unsafe_access(data: &[u8], index: usize) -> u8 {
    unsafe { *data.get_unchecked(index) }
}
