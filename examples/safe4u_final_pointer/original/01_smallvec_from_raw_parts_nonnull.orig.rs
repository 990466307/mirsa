// sample_label: servo.rust-smallvec/src/lib.rs/from_raw_parts/1476
// repo_name: servo.rust-smallvec
// relative_file: src/lib.rs
// function: from_raw_parts
// lines: 1476-1491
// source: /home/wentao/Safe4U-replication/data/manual/manually_reviewed_unsafe.json

pub unsafe fn from_raw_parts(ptr: *mut T, length: usize, capacity: usize) -> SmallVec<T, N> {
        assert!(!Self::is_zst());

        // SAFETY: We require caller to provide same ptr as we alloc
        // and we never alloc null pointer.
        let ptr = unsafe {
            debug_assert!(!ptr.is_null(), "Called `from_raw_parts` with null pointer.");
            NonNull::new_unchecked(ptr)
        };

        SmallVec {
            len: TaggedLen::new(length, true, Self::is_zst()),
            raw: RawSmallVec::new_heap(ptr, capacity),
            _marker: PhantomData,
        }
    }
