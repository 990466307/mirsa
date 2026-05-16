// sample_label: Alexhuszagh.rust-lexical/lexical-util/src/digit.rs/digit_to_char/105
// repo_name: Alexhuszagh.rust-lexical
// relative_file: lexical-util/src/digit.rs
// function: digit_to_char
// lines: 105-113
// source: /home/wentao/Safe4U-replication/data/manual/manually_reviewed_unsafe.json

pub unsafe fn digit_to_char(digit: u32) -> u8 {
    const TABLE: [u8; 36] = [
        b'0', b'1', b'2', b'3', b'4', b'5', b'6', b'7', b'8', b'9', b'A', b'B', b'C', b'D', b'E',
        b'F', b'G', b'H', b'I', b'J', b'K', b'L', b'M', b'N', b'O', b'P', b'Q', b'R', b'S', b'T',
        b'U', b'V', b'W', b'X', b'Y', b'Z',
    ];
    debug_assert!(digit < 36, "digit_to_char() invalid character.");
    unsafe { *TABLE.get_unchecked(digit as usize) }
}
