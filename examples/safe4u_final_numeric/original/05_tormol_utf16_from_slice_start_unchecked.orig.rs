// sample_label: tormol.encode_unicode/src/utf16_char.rs/from_slice_start_unchecked/468
// repo_name: tormol.encode_unicode
// relative_file: src/utf16_char.rs
// function: from_slice_start_unchecked
// lines: 468-477
// source: /home/wentao/Safe4U-replication/data/manual/manually_reviewed_unsafe.json

pub unsafe fn from_slice_start_unchecked(src: &[u16]) -> (Self,usize) {
        unsafe {
            let first = *src.get_unchecked(0);
            if first.is_utf16_leading_surrogate() {
                (Utf16Char{ units: [first, *src.get_unchecked(1)] }, 2)
            } else {
                (Utf16Char{ units: [first, 0] }, 1)
            }
        }
    }
