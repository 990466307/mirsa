mod ast;
mod diagnostic;
mod parser;

pub use ast::*;
pub use diagnostic::{Diagnostic, DiagnosticContext};
pub use parser::{parse, parse_annotation, parse_expression};

pub const GRAMMAR: &str = include_str!("../grammar.ebnf");
