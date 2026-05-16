// sample_label: bitvecto-rs.bitvec/src/slice.rs/split_at_unchecked_mut/902
// repo_name: bitvecto-rs.bitvec
// relative_file: src/slice.rs
// function: split_at_unchecked_mut
// lines: 902-913
// source: /home/wentao/Safe4U-replication/data/manual/manually_reviewed_unsafe.json

pub unsafe fn split_at_unchecked_mut(
		&mut self,
		mid: usize,
	) -> (&mut BitSlice<T::Alias, O>, &mut BitSlice<T::Alias, O>) {
		let len = self.len();
		let left = self.alias_mut().as_mut_bitptr();
		let right = left.add(mid);
		(
			left.span_unchecked(mid).into_bitslice_mut(),
			right.span_unchecked(len - mid).into_bitslice_mut(),
		)
	}
