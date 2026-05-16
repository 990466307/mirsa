// sample_label: magick_rust::MagickWand::get_image_artifacts
// note: reconstructed from magick-rust binding pattern; this item is not present in local Safe4U raw_code.

pub fn get_image_artifacts(&self, pattern: &str) -> Result<Vec<String>, MagickError> {
    let pattern = CString::new(pattern)?;
    let mut num_artifacts = 0usize;
    let c_values = unsafe {
        bindings::MagickGetImageArtifacts(self.wand, pattern.as_ptr(), &mut num_artifacts)
    };
    if c_values.is_null() {
        return Err(self.get_exception().into());
    }

    let mut values = Vec::with_capacity(num_artifacts);
    for i in 0..num_artifacts {
        let c_value = unsafe { *c_values.add(i) };
        let value = unsafe { CStr::from_ptr(c_value) }.to_str()?.to_owned();
        values.push(value);
    }
    unsafe { bindings::MagickRelinquishMemory(c_values.cast()) };
    Ok(values)
}
