pub fn definitely_out_of_bounds() -> u8 {
    let table = [10u8, 20, 30];
    let index = 5usize;
    unsafe { *table.get_unchecked(index) }
}

pub fn bounded_by_constant(data: &[u8]) -> Option<u8> {
    if data.len() >= 4 {
        Some(unsafe { *data.get_unchecked(3) })
    } else {
        None
    }
}

pub fn correctly_guarded(data: &[u8], index: usize) -> u8 {
    if index < data.len() {
        unsafe { *data.get_unchecked(index) }
    } else {
        0
    }
}
