#![allow(dead_code, unused_assignments, unused_mut, unused_variables)]

use std::ptr::NonNull;

fn main() {
    let mut value0 = 1u8;
    let mut value1 = 2u8;
    let mut value2 = 3u8;

    let n0 = std::ptr::null_mut::<u8>();
    let n1 = std::ptr::null_mut::<u8>();
    let p0 = &mut value0 as *mut u8;
    let p1 = &mut value1 as *mut u8;
    let p2 = &mut value2 as *mut u8;

    let a = [n0, p0, n1];
    let b = &a[0];
    let bb = &&b;

    let mut c = p2;
    c = ***bb;
    c = a[1];
    let from_tail = &a[2];
    c = *from_tail;

    let mut d = (a, c);
    d.1 = d.0[1];
    let d1_ref = &d.1;
    let mut saved_nonnull = *d1_ref;
    d.1 = *from_tail;

    let e = &&d.0[1];
    let mut f = **e;
    f = d.1;

    let mut nested = ((d.0, d.1), [f, saved_nonnull, p1]);
    let from_nested_null = &&nested.0 .0[2];
    let mut h = **from_nested_null;
    h = nested.1[1];
    nested.1[1] = h;
    nested.1[2] = **e;
    nested.0 .1 = *from_tail;

    let q_ref = &&nested.1[0];
    let q = **q_ref;

    let mut slots = [p2, p0, p1];
    let slot0 = &mut slots[0];
    *slot0 = q;

    let precise = ((n0, p0), (n1, p1));
    let precise_ref = &&precise.1 .0;
    let precise_null = **precise_ref;

    let _bad = unsafe { NonNull::new_unchecked(precise_null) };
}
