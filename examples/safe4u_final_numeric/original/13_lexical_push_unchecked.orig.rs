// sample_label: Alexhuszagh.rust-lexical/lexical-parse-float/src/bigint.rs/push_unchecked/360
// repo_name: Alexhuszagh.rust-lexical
// relative_file: lexical-parse-float/src/bigint.rs
// function: push_unchecked
// lines: 360-369
// source: /home/wentao/Safe4U-replication/data/manual/manually_reviewed_unsafe.json

pub unsafe fn push_unchecked(&mut self, value: Limb) {
        debug_assert!(self.len() < self.capacity());
        // SAFETY: safe, capacity is less than the current size.
        unsafe {
            let len = self.len();
            let ptr = self.as_mut_ptr().add(len);
            ptr.write(value);
            self.length += 1;
        }
    }
