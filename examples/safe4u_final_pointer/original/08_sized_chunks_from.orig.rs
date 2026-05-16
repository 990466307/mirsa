// sample_label: bodil.sized-chunks-1c55e97df0ab2f78b1c0e205c2ee8cda301c1199/src/sized_chunk/mod.rs/from/818
// repo_name: bodil.sized-chunks-1c55e97df0ab2f78b1c0e205c2ee8cda301c1199
// relative_file: src/sized_chunk/mod.rs
// function: from
// lines: 818-827
// source: /home/wentao/Safe4U-replication/data/checked/11cve.json

fn from(array: &mut InlineArray<A, T>) -> Self {
        let mut out = Self::new();
        out.left = 0;
        out.right = array.len();
        unsafe {
            ptr::copy_nonoverlapping(array.data(), out.mut_ptr(0), out.right);
            *array.len_mut() = 0;
        }
        out
    }
