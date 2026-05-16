// sample_label: tokio-rs.slab/src/lib.rs/get_unchecked_mut/848
// repo_name: tokio-rs.slab
// relative_file: src/lib.rs
// function: get_unchecked_mut
// lines: 848-853
// source: /home/wentao/Safe4U-replication/data/manual/manually_reviewed_unsafe.json

pub unsafe fn get_unchecked_mut(&mut self, key: usize) -> &mut T {
        match *self.entries.get_unchecked_mut(key) {
            Entry::Occupied(ref mut val) => val,
            _ => unreachable!(),
        }
    }
