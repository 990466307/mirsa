// sample_label: Alexhuszagh.rust-lexical/lexical-parse-float/src/bigint.rs/extend_unchecked/414
// repo_name: Alexhuszagh.rust-lexical
// relative_file: lexical-parse-float/src/bigint.rs
// function: extend_unchecked
// lines: 414-425
// source: /home/wentao/Safe4U-replication/data/manual/manually_reviewed_unsafe.json

pub unsafe fn extend_unchecked(&mut self, slc: &[Limb]) {
        let index = self.len();
        let new_len = index + slc.len();
        debug_assert!(self.len() + slc.len() <= self.capacity());
        let src = slc.as_ptr();
        // SAFETY: safe if `self.len() + slc.len() <= self.capacity()`.
        unsafe {
            let dst = self.as_mut_ptr().add(index);
            ptr::copy_nonoverlapping(src, dst, slc.len());
            self.set_len(new_len);
        }
    }
