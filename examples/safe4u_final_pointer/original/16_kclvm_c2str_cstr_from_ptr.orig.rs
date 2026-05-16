// sample_label: kclvm c2str
// note: reconstructed from KCLVM C-string helper pattern; this item is not present in local Safe4U raw_code.

pub fn c2str(p: *const std::os::raw::c_char) -> String {
    if p.is_null() {
        return String::new();
    }
    let s = unsafe { std::ffi::CStr::from_ptr(p) }.to_str().unwrap();
    s.to_string()
}
