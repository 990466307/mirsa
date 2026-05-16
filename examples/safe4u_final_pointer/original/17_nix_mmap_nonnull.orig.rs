// sample_label: nix-rust.nix/src/sys/mman.rs/mmap/394
// repo_name: nix-rust.nix
// relative_file: src/sys/mman.rs
// function: mmap
// lines: 394-416
// source: /home/wentao/Safe4U-replication/data/manual/manually_reviewed_unsafe.json

pub unsafe fn mmap<F: AsFd>(
    addr: Option<NonZeroUsize>,
    length: NonZeroUsize,
    prot: ProtFlags,
    flags: MapFlags,
    f: F,
    offset: off_t,
) -> Result<NonNull<c_void>> {
    let ptr = addr.map_or(std::ptr::null_mut(), |a| a.get() as *mut c_void);

    let fd = f.as_fd().as_raw_fd();
    let ret = unsafe {
        libc::mmap(ptr, length.into(), prot.bits(), flags.bits(), fd, offset)
    };

    if ret == libc::MAP_FAILED {
        Err(Errno::last())
    } else {
        // SAFETY: `libc::mmap` returns a valid non-null pointer or `libc::MAP_FAILED`, thus `ret`
        // will be non-null here.
        Ok(unsafe { NonNull::new_unchecked(ret) })
    }
}
