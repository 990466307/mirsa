#![feature(rustc_private)]

extern crate rustc_driver;
extern crate rustc_hir;
extern crate rustc_interface;
extern crate rustc_middle;

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

struct MirCallbacks;

impl Callbacks for MirCallbacks {
    fn after_analysis(
        &mut self,
        _compiler: &rustc_interface::interface::Compiler,
        tcx: TyCtxt<'_>,
    ) -> Compilation {
        let fns = core::collect::collect_local_fns(tcx);

        for def_id in fns {
            let body = core::mir::get_optimized_mir(tcx, def_id);
            let cfg = core::cfg::build_cfg(body);
            let places = core::mir::collect_body_places(tcx, body);

            // // 获取符号分析结果
            // domains::sign::run_sign(tcx, def_id, body, &cfg, &places);
            // // 打印 MIR 结构（可选）
            // print_mir_simple(tcx, def_id, body);

            // 运行并打印区间分析结果
            domains::internval::run_internval(tcx, def_id, body, &cfg, &places);
        }

        Compilation::Continue
    }
}

fn normalize_rustc_args(mut args: Vec<String>) -> Vec<String> {
    if !args.is_empty() {
        args[0] = "rustc".to_string();
    }

    let has_emit = args.iter().any(|arg| arg == "--emit" || arg.starts_with("--emit="));
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

fn main() {
    let args = normalize_rustc_args(std::env::args().collect());
    let mut callbacks = MirCallbacks;
    run_compiler(&args, &mut callbacks);
}
