// sample_label: bitvecto-rs.bitvec/src/slice.rs/split_at_unchecked/876
// repo_name: bitvecto-rs.bitvec
// relative_file: src/slice.rs
// function: split_at_unchecked
// lines: 876-885
// source: /home/wentao/Safe4U-replication/data/manual/manually_reviewed_unsafe.json

pub unsafe fn split_at_unchecked(&self, mid: usize) -> (&Self, &Self) {
		let len = self.len();
		let left = self.as_bitptr();
		let right = left.add(mid);
		let left = left.span_unchecked(mid);
		let right = right.span_unchecked(len - mid);
		let left = left.into_bitslice_ref();
		let right = right.into_bitslice_ref();
		(left, right)
	}
