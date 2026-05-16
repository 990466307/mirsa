// sample_label: petgraph.fixedbitset/src/lib.rs/copy_bit/550
// repo_name: petgraph.fixedbitset
// relative_file: src/lib.rs
// function: copy_bit
// lines: 550-560
// source: /home/wentao/Safe4U-replication/data/checked/risky.json

pub fn copy_bit(&mut self, from: usize, to: usize) {
        assert!(
            to < self.length,
            "copy to index {} exceeds fixedbitset size {}",
            to,
            self.length
        );
        let enabled = self.contains(from);
        // SAFETY: The above assertion ensures that the block is inside the Vec's allocation.
        unsafe { self.set_unchecked(to, enabled) };
    }
