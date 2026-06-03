#![allow(dead_code, unused_variables)]
// use std::num::NonZero;
use std::ptr::NonNull;
use core::alloc::Layout;
fn main() {
    let a = [
        std::ptr::null_mut::<u8>(),
        std::ptr::null_mut::<u8>(),
        std::ptr::null_mut::<u8>(),
    ];
    let b = &a[0];
    let mut value = 1u8;
    let mut c = &mut value as *mut u8;
    if a[0] == *b {
        c = *b;
    }
    let mut d = (a, c);
    d.1 = *b;
    let e = &&d.0[1];
    let f = **e;
    let _bad = unsafe { NonNull::new_unchecked(c) };
    let layout = unsafe { Layout::from_size_align_unchecked(16, 8) };
    // let mut x = 0;
    // let p = &mut x;
    // *p = 1;
    // if x == 0 {
    //     let _bad = unsafe { NonZero::new_unchecked(0) };
    // }
}
