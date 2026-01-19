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
/// - locals 数量
/// - basic block 数量
/// - 每个 BB 的 statements / terminator / successors
pub fn print_mir_simple<'tcx>(tcx: TyCtxt<'tcx>, def_id: DefId, body: &Body<'tcx>) {
    let name = tcx.def_path_str(def_id);
    println!("== fn: {name} ==");

    println!("locals: {}", body.local_decls.len());
    println!("basic_blocks: {}", body.basic_blocks.len());

    for (bb, bbdata) in body.basic_blocks.iter_enumerated() {
        println!("  bb{}:", bb.index());

        for stmt in &bbdata.statements {
            println!("    stmt: {:?}", stmt.kind);
        }

        if let Some(term) = &bbdata.terminator {
            println!("    term: {:?}", term.kind);

            // successors（对 CFG / 数据流 debug 非常有用）
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
            print_mir_simple(tcx, def_id, body);
            let cfg = core::cfg::build_cfg(body);

            println!("\n== fn: {} ==", tcx.def_path_str(def_id));

            let res = domains::sign::run_sign(tcx, body, &cfg);

            for (bb, _bbdata) in body.basic_blocks.iter_enumerated() {
                println!(
                    "  OUT[BB{}] = {:?}",
                    bb.index(),
                    res.out_states[bb.index()].locals
                );
            }
        }

        Compilation::Continue
    }
}

fn main() {
    let mut args: Vec<String> = std::env::args().collect();
    if !args.is_empty() {
        args[0] = "rustc".to_string(); // 更稳
    }
    let mut callbacks = MirCallbacks;
    run_compiler(&args, &mut callbacks);
}
