#![feature(rustc_private)]
use mirsa::{Mirsa, SourceBindings, query::Report};

fn main() -> Result<(), mirsa::Error> {
    let source = "examples/inputs/query_target.rs";
    let bindings: SourceBindings = SourceBindings::new()
        .variable("slice", "s") // DSL 的 $slice 对应源码变量 s: &[u8]
        .variable("index", "i"); // DSL 的 $index 对应源码变量 i: usize

    // 性质查询：下标是否在范围内。
    let check: Report =
        Mirsa::from_file(source).query("$index < len($slice) at 'access", &bindings)?;
    println!("性质结果：\n{check}");

    // 信息查询：切片的抽象信息及相关符号关系。
    let info: Report = Mirsa::from_file(source).query("facts($slice) at 'access", &bindings)?;
    println!("抽象信息：\n{info}");
    Ok(())
}
