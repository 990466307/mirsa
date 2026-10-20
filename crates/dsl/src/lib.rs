mod ast;
mod parser;

pub use ast::*;
pub use parser::{parse, parse_expression};

pub const GRAMMAR: &str = include_str!("../grammar.ebnf");
