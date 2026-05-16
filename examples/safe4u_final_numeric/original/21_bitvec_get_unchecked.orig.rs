// sample_label: bitvecto-rs.bitvec/src/slice/api.rs/get_unchecked/479
// repo_name: bitvecto-rs.bitvec
// relative_file: src/slice/api.rs
// function: get_unchecked

pub unsafe fn get_unchecked<'a, I>(&'a self, index: I) -> I::Immut
	where I: BitSliceIndex<'a, T, O> {
		index.get_unchecked(self)
	}
