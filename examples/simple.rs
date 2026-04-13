#![allow(dead_code, unused_variables)]

fn main() {
    let a = 1i32;
    let b = 2i32;
    let mut c = a + b;
    let d = c - 3;

    if d == 0 {
        c = c + 10;
    } else {
        c = c - 10;
    }

    let mut arr = [0i32, 1, 2];
    let idx = 1usize;
    arr[idx] = c;
    let e = arr[idx];

    if e > 0 {
        c = e + 3;
    } else {
        c = e - 3;
    }

    let f = c as i64;
    let g = f as i32;
}
