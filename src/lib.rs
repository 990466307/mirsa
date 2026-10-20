#![feature(rustc_private)]

pub use mirsa_analysis as analysis;
pub use mirsa_contracts as contracts;
pub use mirsa_core as core;
pub use mirsa_domains as domains;
pub use mirsa_framework as framework;
pub use mirsa_relations as relations;

pub use mirsa_analysis::query;

extern crate rustc_driver;
extern crate rustc_hir;
extern crate rustc_interface;
extern crate rustc_lexer;
extern crate rustc_middle;
extern crate rustc_session;
extern crate rustc_span;

mod input;
pub use input::{Error, MirInput, Mirsa, Options, SourceBindings, SourceInput};
