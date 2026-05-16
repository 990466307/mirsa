// sample_label: ferrilab.ferrilab/funty/src/ptr.rs/new_unchecked/1050
// repo_name: ferrilab.ferrilab
// relative_file: funty/src/ptr.rs
// function: new_unchecked

pub const unsafe fn new_unchecked(ptr: *const T) -> Self {
		Self::from_nonnull(NonNull::new_unchecked(ptr.cast_mut()))
	}
