// sample_label: bytecodealliance.rustix/src/backend/libc/event/epoll.rs/from_raw_parts/423
// repo_name: bytecodealliance.rustix
// relative_file: src/backend/libc/event/epoll.rs
// function: from_raw_parts
// lines: 423-427
// source: /home/wentao/Safe4U-replication/data/manual/manually_reviewed_unsafe.json

pub unsafe fn from_raw_parts(ptr: *mut Event, len: usize, capacity: usize) -> Self {
        Self {
            events: Vec::from_raw_parts(ptr, len, capacity),
        }
    }
