fn main() {
    let a: i32 = 5;
    let b: i32 = -2;
    let c = a + 1;   // Pos
    let d = b * 3;   // Neg
    let e = c + d;   // Top（正 + 负）
    let f = -d;      // Pos
    let _ = (e, f);
}
