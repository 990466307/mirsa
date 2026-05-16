// sample_label: nix-rust.nix/src/sys/mman.rs/mremap/458
// repo_name: nix-rust.nix
// relative_file: src/sys/mman.rs
// function: mremap
// lines: 458-497
// source: /home/wentao/Safe4U-replication/data/manual/manually_reviewed_unsafe.json

pub unsafe fn mremap(
    addr: NonNull<c_void>,
    old_size: size_t,
    new_size: size_t,
    flags: MRemapFlags,
    new_address: Option<NonNull<c_void>>,
) -> Result<NonNull<c_void>> {
    #[cfg(target_os = "linux")]
    let ret = unsafe {
        libc::mremap(
            addr.as_ptr(),
            old_size,
            new_size,
            flags.bits(),
            new_address
                .map(NonNull::as_ptr)
                .unwrap_or(std::ptr::null_mut()),
        )
    };
    #[cfg(target_os = "netbsd")]
    let ret = unsafe {
        libc::mremap(
            addr.as_ptr(),
            old_size,
            new_address
                .map(NonNull::as_ptr)
                .unwrap_or(std::ptr::null_mut()),
            new_size,
            flags.bits(),
        )
    };

    if ret == libc::MAP_FAILED {
        Err(Errno::last())
    } else {
        // SAFETY: `libc::mremap` returns a valid non-null pointer or `libc::MAP_FAILED`, thus `ret`
        // will be non-null here.
        Ok(unsafe { NonNull::new_unchecked(ret) })
    }
}
