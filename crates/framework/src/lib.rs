#![feature(rustc_private)]

extern crate rustc_driver;
extern crate rustc_hir;
extern crate rustc_middle;
extern crate rustc_span;

pub mod access_path;
pub mod config;
pub mod eq_domain;
pub mod forward;
pub mod printer;
