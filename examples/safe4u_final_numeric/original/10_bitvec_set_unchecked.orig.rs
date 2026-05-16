// sample_label: bitvecto-rs.bitvec/src/slice.rs/set_unchecked/786
// repo_name: bitvecto-rs.bitvec
// relative_file: src/slice.rs
// function: set_unchecked
// lines: 786-788
// source: /home/wentao/Safe4U-replication/data/manual/manually_reviewed_unsafe.json

pub unsafe fn set_unchecked(&mut self, index: usize, value: bool) {
		self.replace_unchecked(index, value);
	}
