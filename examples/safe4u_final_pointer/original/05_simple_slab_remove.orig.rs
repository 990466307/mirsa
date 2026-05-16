// sample_label: nathansizemore.simple-slab-5e0524c1db836e2192e1cd818848d96937c0b587/src/lib.rs/remove/82
// repo_name: nathansizemore.simple-slab-5e0524c1db836e2192e1cd818848d96937c0b587
// relative_file: src/lib.rs
// function: remove
// lines: 82-102
// source: /home/wentao/Safe4U-replication/data/checked/11cve.json

pub fn remove(&mut self, offset: usize) -> T {
        assert!(offset < self.len, "Offset out of bounds");

        let elem: T;
        let last_elem: T;
        let elem_ptr: *mut T;
        let last_elem_ptr: *mut T;

        unsafe {
            elem_ptr = self.mem.offset(offset as isize);
            last_elem_ptr = self.mem.offset(self.len as isize);

            elem = ptr::read(elem_ptr);
            last_elem = ptr::read(last_elem_ptr);

            ptr::write(elem_ptr, last_elem);
        }

        self.len -= 1;
        return elem;
    }
