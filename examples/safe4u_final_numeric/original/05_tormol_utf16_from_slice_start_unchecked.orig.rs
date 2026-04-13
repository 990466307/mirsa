// sample_label: tormol-encode-unicode/src/utf16_char.rs/from_slice_start_unchecked/468
//
// let first = *src.get_unchecked(0);
// if first.is_leading_surrogate() {
//     (Utf16Char { units: [first, *src.get_unchecked(1)] }, 2)
// }
