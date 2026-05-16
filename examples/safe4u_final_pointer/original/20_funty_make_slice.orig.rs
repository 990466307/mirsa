// sample_label: ferrilab.ferrilab/funty/src/ptr.rs/make_slice/858
// repo_name: ferrilab.ferrilab
// relative_file: funty/src/ptr.rs
// function: make_slice

pub unsafe fn make_slice(self, len: usize) -> Pointer<[T], P> {
		Pointer::from_raw_parts(self, len)
	}
