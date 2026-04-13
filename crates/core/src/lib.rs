#![feature(rustc_private)]

extern crate rustc_driver;
extern crate rustc_hir;
extern crate rustc_middle;

pub mod cfg;
pub mod collect;
pub mod mir;
