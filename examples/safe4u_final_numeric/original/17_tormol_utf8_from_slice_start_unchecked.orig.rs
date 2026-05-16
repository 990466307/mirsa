// sample_label: tormol.encode_unicode/src/utf8_char.rs/from_slice_start_unchecked/456
// repo_name: tormol.encode_unicode
// relative_file: src/utf8_char.rs
// function: from_slice_start_unchecked

pub unsafe fn from_slice_start_unchecked(src: &[u8]) -> (Self,usize) {
        unsafe {
            let len = 1+src.get_unchecked(0).extra_utf8_bytes_unchecked();
            let mut bytes = [0; 4];
            ptr::copy_nonoverlapping(src.as_ptr(), bytes.as_mut_ptr() as *mut u8, len);
            (Utf8Char{bytes}, len)
        }
    }
