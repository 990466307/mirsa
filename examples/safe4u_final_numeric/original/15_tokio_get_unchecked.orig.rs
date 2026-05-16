// sample_label: tokio-rs.slab/src/lib.rs/get_unchecked/816
// repo_name: tokio-rs.slab
// relative_file: src/lib.rs
// function: get_unchecked
// lines: 816-821
// source: /home/wentao/Safe4U-replication/data/manual/manually_reviewed_unsafe.json

pub unsafe fn get_unchecked(&self, key: usize) -> &T {
        match *self.entries.get_unchecked(key) {
            Entry::Occupied(ref val) => val,
            _ => unreachable!(),
        }
    }
