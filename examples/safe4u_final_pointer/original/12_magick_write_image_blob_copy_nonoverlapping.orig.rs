// sample_label: magick_rust::MagickWand::write_image_blob
// note: reconstructed from magick-rust binding pattern; this item is not present in local Safe4U raw_code.

pub fn write_image_blob(&self, format: &str) -> Result<Vec<u8>, MagickError> {
    self.set_image_format(format)?;
    let mut length = 0usize;
    let blob = unsafe { bindings::MagickWriteImageBlob(self.wand, &mut length) };
    if blob.is_null() {
        return Err(self.get_exception().into());
    }

    let mut bytes = Vec::with_capacity(length);
    unsafe {
        ptr::copy_nonoverlapping(blob, bytes.as_mut_ptr(), length);
        bytes.set_len(length);
        bindings::MagickRelinquishMemory(blob.cast());
    }
    Ok(bytes)
}
