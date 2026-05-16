// sample_label: hsivonen.encoding_rs/src/single_byte.rs/latin1_byte_compatible_up_to/254
// repo_name: hsivonen.encoding_rs
// relative_file: src/single_byte.rs
// function: latin1_byte_compatible_up_to
// lines: 254-272
// source: /home/wentao/Safe4U-replication/data/checked/risky.json

pub fn latin1_byte_compatible_up_to(&self, buffer: &[u8]) -> usize {
        let mut bytes = buffer;
        let mut total = 0;
        loop {
            if let Some((non_ascii, offset)) = validate_ascii(bytes) {
                total += offset;
                // Safety: We can rely on `non_ascii` being between `0x80` and `0xFF` due to
                // the invariants of `ascii_to_basic_latin()`, and our table has enough space for that.
                let mapped = unsafe { *(self.table.get_unchecked(non_ascii as usize - 0x80usize)) };
                if mapped != u16::from(non_ascii) {
                    return total;
                }
                total += 1;
                bytes = &bytes[offset + 1..];
            } else {
                return total;
            }
        }
    }
