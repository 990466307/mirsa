// sample_label: bitvecto-rs.bitvec/src/slice.rs/copy_within_unchecked/949
// repo_name: bitvecto-rs.bitvec
// relative_file: src/slice.rs
// function: copy_within_unchecked
// lines: 949-969
// source: /home/wentao/Safe4U-replication/data/manual/manually_reviewed_unsafe.json

pub unsafe fn copy_within_unchecked<R>(&mut self, src: R, dest: usize)
	where R: RangeExt<usize> {
		if let Some(this) = self.coerce_mut::<T, Lsb0>() {
			return this.sp_copy_within_unchecked(src, dest);
		}
		if let Some(this) = self.coerce_mut::<T, Msb0>() {
			return this.sp_copy_within_unchecked(src, dest);
		}
		let source = src.normalize(0, self.len());
		let source_len = source.len();
		let rev = source.contains(&dest);
		let dest = dest .. dest + source_len;
		for (from, to) in self
			.get_unchecked(source)
			.as_bitptr_range()
			.zip(self.get_unchecked_mut(dest).as_mut_bitptr_range())
			.bidi(rev)
		{
			to.write(from.read());
		}
	}
