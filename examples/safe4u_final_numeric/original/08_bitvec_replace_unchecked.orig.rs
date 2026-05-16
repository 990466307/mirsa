// sample_label: bitvecto-rs.bitvec/src/slice.rs/replace_unchecked/832
// repo_name: bitvecto-rs.bitvec
// relative_file: src/slice.rs
// function: replace_unchecked
// lines: 832-838
// source: /home/wentao/Safe4U-replication/data/manual/manually_reviewed_unsafe.json

pub unsafe fn replace_unchecked(
		&mut self,
		index: usize,
		value: bool,
	) -> bool {
		self.as_mut_bitptr().add(index).replace(value)
	}
