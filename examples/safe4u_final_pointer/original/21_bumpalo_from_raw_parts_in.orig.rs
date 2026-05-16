// sample_label: fitzgen.bumpalo/src/collections/string.rs/from_raw_parts_in/763
// repo_name: fitzgen.bumpalo
// relative_file: src/collections/string.rs
// function: from_raw_parts_in

pub unsafe fn from_raw_parts_in(
        buf: *mut u8,
        length: usize,
        capacity: usize,
        bump: &'bump Bump,
    ) -> String<'bump> {
        String {
            vec: Vec::from_raw_parts_in(buf, length, capacity, bump),
        }
    }
