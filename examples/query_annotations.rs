#![feature(rustc_private)]
use mirsa::Mirsa;

fn main() -> Result<(), mirsa::Error> {
    let source = r#"
pub fn example(data: &[u8], index: usize) -> u8 {
    if index < data.len() {
        #[mirsa::check(index < len(data))]
        #[mirsa::facts(data)]
        unsafe { *data.get_unchecked(index) }
    } else { 0 }
}
"#;
    for query in Mirsa::from_source(source).analyze_annotations()? {
        println!(
            "{}:{}:{} {}",
            query.file, query.line, query.column, query.query
        );
        println!("{}", query.report);
    }
    Ok(())
}
