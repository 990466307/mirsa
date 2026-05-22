use rustc_middle::mir::{Body, Terminator, TerminatorKind};
use rustc_middle::ty::TyCtxt;

use crate::contracts::finding::{Finding, Level};
use crate::internval::{Internval, InternvalState};

use super::shared::{check_le, eval_call_arg, pointer_len_from_operand};

fn check_side(count: Internval, available: Option<Internval>) -> Level {
    let Some(available) = available else {
        return Level::Safe;
    };
    check_le(count, available)
}

pub(crate) fn check<'tcx>(
    tcx: TyCtxt<'tcx>,
    body: &Body<'tcx>,
    term: &Terminator<'tcx>,
    state: &InternvalState<'tcx>,
) -> Option<Finding> {
    let TerminatorKind::Call { args, .. } = &term.kind else {
        return None;
    };
    if args.len() < 3 {
        return None;
    }

    let src_len = pointer_len_from_operand(tcx, body, state, &args[0].node, 8);
    let dst_len = pointer_len_from_operand(tcx, body, state, &args[1].node, 8);
    let count = eval_call_arg(tcx, body, state, &args[2].node);
    let src_level = check_side(count, src_len);
    let dst_level = check_side(count, dst_len);
    let level = src_level.combine(dst_level);
    let src_note = src_len
        .map(|len| format!("src_len = {len}"))
        .unwrap_or_else(|| "src_len = unknown".to_string());
    let dst_note = dst_len
        .map(|len| format!("dst_len = {len}"))
        .unwrap_or_else(|| "dst_len = unknown".to_string());
    Finding::for_level(
        level,
        term.source_info.span,
        "internval/definite-oob",
        "internval/possible-oob",
        "calling `ptr::copy_nonoverlapping` with a definitely out-of-bounds range",
        "calling `ptr::copy_nonoverlapping` with a range that may exceed the source or destination object",
        vec![format!("count = {count}"), src_note, dst_note],
    )
}
