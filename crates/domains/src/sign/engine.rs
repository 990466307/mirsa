use std::collections::VecDeque;

use rustc_middle::mir::*;
use rustc_middle::ty::TyCtxt;

use core::cfg::Cfg;

use super::state::{join_state, SignState};
use super::transfer::transfer_block;

#[derive(Clone, Debug)]
pub struct SignAnalysisResult {
    pub in_states: Vec<SignState>,
    pub out_states: Vec<SignState>,
}

pub fn run_sign<'tcx>(tcx: TyCtxt<'tcx>, body: &Body<'tcx>, cfg: &Cfg) -> SignAnalysisResult {
    let n = body.basic_blocks.len();
    let mut in_states = vec![SignState::default(); n];
    let mut out_states = vec![SignState::default(); n];

    let mut worklist = VecDeque::new();
    worklist.push_back(BasicBlock::from_usize(0));

    while let Some(bb) = worklist.pop_front() {
        // 合流：in[bb] = join(out[pred...])
        if bb.index() != 0 {
            let mut merged: Option<SignState> = None;
            for p in &cfg.pred[bb.index()] {
                let po = &out_states[p.index()];
                merged = Some(match merged {
                    None => po.clone(),
                    Some(acc) => join_state(&acc, po),
                });
            }
            if let Some(new_in) = merged {
                if new_in != in_states[bb.index()] {
                    in_states[bb.index()] = new_in;
                }
            }
        }

        let new_out = transfer_block(tcx, body, bb, &in_states[bb.index()]);
        if new_out != out_states[bb.index()] {
            out_states[bb.index()] = new_out;
            for s in &cfg.succ[bb.index()] {
                worklist.push_back(*s);
            }
        }
    }

    SignAnalysisResult { in_states, out_states }
}
