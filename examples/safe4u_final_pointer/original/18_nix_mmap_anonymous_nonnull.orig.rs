// sample_label: nix-rust.nix/src/sys/mman.rs/mmap_anonymous/428
// repo_name: nix-rust.nix
// relative_file: src/sys/mman.rs
// function: mmap_anonymous
// lines: 428-448
// source: /home/wentao/Safe4U-replication/data/manual/manually_reviewed_unsafe.json

pub unsafe fn mmap_anonymous(
    addr: Option<NonZeroUsize>,
    length: NonZeroUsize,
    prot: ProtFlags,
    flags: MapFlags,
) -> Result<NonNull<c_void>> {
    let ptr = addr.map_or(std::ptr::null_mut(), |a| a.get() as *mut c_void);

    let flags = MapFlags::MAP_ANONYMOUS | flags;
    let ret = unsafe {
        libc::mmap(ptr, length.into(), prot.bits(), flags.bits(), -1, 0)
    };

    if ret == libc::MAP_FAILED {
        Err(Errno::last())
    } else {
        // SAFETY: `libc::mmap` returns a valid non-null pointer or `libc::MAP_FAILED`, thus `ret`
        // will be non-null here.
        Ok(unsafe { NonNull::new_unchecked(ret) })
    }
}
