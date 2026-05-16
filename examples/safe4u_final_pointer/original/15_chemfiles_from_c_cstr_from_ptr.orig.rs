// sample_label: chemfiles/strings.rs/from_c
// source: https://chemfiles.org/chemfiles.rs/latest/src/chemfiles/strings.rs.html

/// Create a Rust string from a C string. Clones all characters in `buffer`.
pub fn from_c(buffer: *const c_char) -> String {
    unsafe {
        let rust_str = CStr::from_ptr(buffer).to_str().expect("Invalid Rust string from C");
        return String::from(rust_str);
    }
}
