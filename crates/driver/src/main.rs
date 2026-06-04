#![feature(rustc_private)]

extern crate rustc_driver;
extern crate rustc_hir;
extern crate rustc_interface;
extern crate rustc_middle;

use mirsa_analysis::reduced_product::AnalysisOptions;
use mirsa_core::mir::{collect_body_places, collect_interval_places};

use rustc_driver::{Callbacks, Compilation, run_compiler};
use rustc_middle::ty::TyCtxt;

use rustc_hir::def_id::DefId;
use rustc_middle::mir::Body;

/// 打印 MIR：
/// - 函数名
/// - 每个 BB 的 statements / terminator / successors
pub fn print_mir_simple<'tcx>(tcx: TyCtxt<'tcx>, def_id: DefId, body: &Body<'tcx>) {
    let name = tcx.def_path_str(def_id);
    println!("== fn: {name} ==");

    for (bb, bbdata) in body.basic_blocks.iter_enumerated() {
        println!("  bb{}:", bb.index());

        for stmt in &bbdata.statements {
            println!("    stmt: {:?}", stmt.kind);
        }

        if let Some(term) = &bbdata.terminator {
            println!("    term: {:?}", term.kind);

            let succs: Vec<_> = term.successors().map(|b| b.index()).collect();
            println!("    succs: {:?}", succs);
        }
    }
}

struct MirCallbacks {
    options: AnalysisOptions,
}

impl Callbacks for MirCallbacks {
    fn after_analysis(
        &mut self,
        _compiler: &rustc_interface::interface::Compiler,
        tcx: TyCtxt<'_>,
    ) -> Compilation {
        let fns = mirsa_core::collect::collect_local_fns(tcx);

        for def_id in fns {
            let body = mirsa_core::mir::get_optimized_mir(tcx, def_id);
            let cfg = mirsa_core::cfg::build_cfg(body);
            let all_places = collect_body_places(tcx, body);
            let places = collect_interval_places(tcx, body);
            let ptr_places = mirsa_core::mir::collect_ptr_places(tcx, body);
            mirsa_analysis::reduced_product::run_combined(
                tcx,
                def_id,
                body,
                &cfg,
                &places,
                &all_places,
                &ptr_places,
                self.options,
            );
        }

        Compilation::Continue
    }
}

fn normalize_rustc_args(mut args: Vec<String>) -> Vec<String> {
    if !args.is_empty() {
        args[0] = "rustc".to_string();
    }

    let has_emit = args
        .iter()
        .any(|arg| arg == "--emit" || arg.starts_with("--emit="));
    let has_out_dir = args
        .iter()
        .any(|arg| arg == "--out-dir" || arg.starts_with("--out-dir="));

    if !has_emit {
        // Run analysis without generating link-time executables by default.
        args.push("--emit=metadata".to_string());
    }
    if !has_out_dir {
        // Keep compiler artifacts under target/ instead of the workspace root.
        args.push("--out-dir".to_string());
        args.push("target/mir-framework-artifacts".to_string());
    }

    args
}

fn parse_driver_args(args: Vec<String>) -> (AnalysisOptions, Vec<String>) {
    let mut options = AnalysisOptions::default();
    let mut filtered = Vec::with_capacity(args.len());
    let mut i = 0usize;
    while i < args.len() {
        match args[i].as_str() {
            "--debug" => {
                options.debug = true;
                i += 1;
                continue;
            }
            "--no-debug" => {
                options.debug = false;
                i += 1;
                continue;
            }
            "--domain" => {
                eprintln!(
                    "error: `--domain` is no longer supported; full-domain analysis is always enabled"
                );
                std::process::exit(2);
            }
            _ => {}
        }
        filtered.push(args[i].clone());
        i += 1;
    }
    (options, filtered)
}

fn main() {
    let (options, args) = parse_driver_args(std::env::args().collect());
    let args = normalize_rustc_args(args);
    let mut callbacks = MirCallbacks { options };
    run_compiler(&args, &mut callbacks);
}
