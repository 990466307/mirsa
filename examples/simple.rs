#![allow(dead_code, unused_variables)]

use std::ptr::NonNull;

struct MyStruct {
    value: i32,
}

fn main() {
    let raw_ptr: *mut MyStruct = std::ptr::null_mut(); // 创建一个空指针
    let p: *mut MyStruct = 0 as *mut MyStruct;
    let q = p.clone();
    let r = &raw_ptr;
    let w = *r;
    let x = 32;
    let ptr: *const i32 = &x;
    let non_null_ptr1 = unsafe { NonNull::new_unchecked(raw_ptr) };
    // let non_null_ptr2 = unsafe { NonNull::new_unchecked(p) };
    // let non_null_ptr3 = unsafe { NonNull::new_unchecked(q) };
}
