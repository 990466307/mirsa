#![feature(rustc_private)]

extern crate rustc_driver;
extern crate rustc_hir;
extern crate rustc_middle;
extern crate rustc_span;

mod combined_warnings;
pub mod reduced_product;

pub use reduced_product as combined;
