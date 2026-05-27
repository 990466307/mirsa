#![allow(dead_code, unused_assignments, unused_mut, unused_variables)]
use std::alloc::Layout
use std::mem::MaybeUninit;
unsafe fn assume_uninit_value() -> i32 {
    let slot = MaybeUninit::<i32>::uninit();
    unsafe { slot.assume_init() }
}

unsafe fn assume_written_value() -> i32 {
    let mut slot = MaybeUninit::<i32>::uninit();
    slot.write(42);
    unsafe { slot.assume_init() }
}

fn from(mut buffer: Vec<u8>) -> Vec<u8> {
    let len = buffer.len();
    let cap = buffer.capacity();
    let ptr = buffer.as_mut_ptr();

    unsafe { Vec::from_raw_parts(ptr, len, cap) }
}

fn main() {
    let b: Vec<u8> = vec![0, 0, 0];
    let f = from(b);
    let _bad = unsafe { assume_uninit_value() };
    let _ok = unsafe { assume_written_value() };
    let _ = unsafe { Layout::from_size_align_unchecked(8, 0)};
}
