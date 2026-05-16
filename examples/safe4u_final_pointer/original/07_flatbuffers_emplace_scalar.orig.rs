// sample_label: google.flatbuffers-7829fe179452ca778812d2338e838f2f9ae133f9/rust/flatbuffers/src/endian_scalar.rs/emplace_scalar/151
// repo_name: google.flatbuffers-7829fe179452ca778812d2338e838f2f9ae133f9/rust/flatbuffers
// relative_file: src/endian_scalar.rs
// function: emplace_scalar
// lines: 151-160
// source: /home/wentao/Safe4U-replication/data/checked/11cve.json

pub fn emplace_scalar<T: EndianScalar>(s: &mut [u8], x: T) {
    let x_le = x.to_little_endian();
    unsafe {
        core::ptr::copy_nonoverlapping(
            &x_le as *const T as *const u8,
            s.as_mut_ptr() as *mut u8,
            size_of::<T>(),
        );
    }
}
