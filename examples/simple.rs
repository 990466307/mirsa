// fn fun() {
//     let mut v = [0, 0, 0, 0, 0];
//     let mut i = 0;
//     let mut j = 0;
//     while i < 5 {
//         v[i] = i;
//         j += v[i];
//         i += 1;
//     }
// }

fn fun(){
    let mut i: i32 = 1;
    let mut s: i32 = i;

    while i < 40 {
        s = s + i;
        i = i + 1;
    }
}

fn main() {
    fun();
}
