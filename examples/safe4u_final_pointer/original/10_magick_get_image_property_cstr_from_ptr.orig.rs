// sample_label: magick_rust::MagickWand::get_image_property
// note: reconstructed from magick-rust binding pattern; this item is not present in local Safe4U raw_code.

pub fn get_image_property(&self, name: &str) -> Result<String, MagickError> {
    let name = CString::new(name)?;
    let c_value = unsafe { bindings::MagickGetImageProperty(self.wand, name.as_ptr()) };
    if c_value.is_null() {
        return Err(self.get_exception().into());
    }

    let value = unsafe { CStr::from_ptr(c_value) }.to_str()?.to_owned();
    unsafe { bindings::MagickRelinquishMemory(c_value.cast()) };
    Ok(value)
}
