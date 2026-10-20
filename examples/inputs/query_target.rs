pub fn example(data: &[u8], index: usize) -> u8 {
    let s = data;
    let i = index;
    if i < s.len() {
        // mirsa: access
        let value = unsafe { *s.get_unchecked(i) };
        value
    } else {
        0
    }
}
