// sample_label: google.flatbuffers-77991e92337bb7474c439e1f579648e4e42cb595/rust/flatbuffers/src/endian_scalar.rs/read_scalar/170
// repo_name: google.flatbuffers-77991e92337bb7474c439e1f579648e4e42cb595/rust/flatbuffers
// relative_file: src/endian_scalar.rs
// function: read_scalar
// lines: 170-178
// source: /home/wentao/Safe4U-replication/data/checked/11cve.json

pub fn read_scalar<T: EndianScalar>(s: &[u8]) -> T {
    let mut mem = core::mem::MaybeUninit::<T>::uninit();
    // Since [u8] has alignment 1, we copy it into T which may have higher alignment.
    let x = unsafe {
        core::ptr::copy_nonoverlapping(s.as_ptr(), mem.as_mut_ptr() as *mut u8, size_of::<T>());
        mem.assume_init()
    };
    x.from_little_endian()
}
