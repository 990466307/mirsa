// sample_label: bitvecto-rs.bitvec/src/slice/api.rs/get_unchecked_mut/521
// repo_name: bitvecto-rs.bitvec
// relative_file: src/slice/api.rs
// function: get_unchecked_mut

pub unsafe fn get_unchecked_mut<'a, I>(&'a mut self, index: I) -> I::Mut
	where I: BitSliceIndex<'a, T, O> {
		index.get_unchecked_mut(self)
	}
