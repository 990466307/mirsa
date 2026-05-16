// sample_label: magick_rust::MagickWand::get_image_artifact
// note: reconstructed from magick-rust binding pattern; this item is not present in local Safe4U raw_code.

pub fn get_image_artifact(&self, artifact: &str) -> Result<String, MagickError> {
    let artifact = CString::new(artifact)?;
    let c_value = unsafe { bindings::MagickGetImageArtifact(self.wand, artifact.as_ptr()) };
    if c_value.is_null() {
        return Err(self.get_exception().into());
    }

    let value = unsafe { CStr::from_ptr(c_value) }.to_str()?.to_owned();
    unsafe { bindings::MagickRelinquishMemory(c_value.cast()) };
    Ok(value)
}
