// sample_label: Alexhuszagh.rust-lexical/lexical-parse-float/src/bigint.rs/get_unchecked/718
// repo_name: Alexhuszagh.rust-lexical
// relative_file: lexical-parse-float/src/bigint.rs
// function: get_unchecked
// lines: 718-722
// source: /home/wentao/Safe4U-replication/data/manual/manually_reviewed_unsafe.json

pub unsafe fn get_unchecked(&self, index: usize) -> &T {
        debug_assert!(index < self.inner.len());
        let len = self.inner.len();
        unsafe { self.inner.get_unchecked(len - index - 1) }
    }
