use rustc_middle::mir::{Body, Terminator, TerminatorKind};
use rustc_middle::ty::TyCtxt;

use crate::internval::{Internval, InternvalState};

use super::shared::{CheckLevel, check_le, emit_warning, eval_call_arg, pointer_len_from_operand};

pub(crate) fn matches_path(path: &str) -> bool {
    path.ends_with("::copy_nonoverlapping")
}

fn check_side(count: Internval, available: Option<Internval>) -> CheckLevel {
    let Some(available) = available else {
        return CheckLevel::Safe;
    };
    check_le(count, available)
}

pub(crate) fn emit<'tcx>(
    tcx: TyCtxt<'tcx>,
    body: &Body<'tcx>,
    term: &Terminator<'tcx>,
    state: &InternvalState<'tcx>,
) {
    let TerminatorKind::Call { args, .. } = &term.kind else {
        return;
    };
    if args.len() < 3 {
        return;
    }

    let src_len = pointer_len_from_operand(tcx, body, state, &args[0].node, 8);
    let dst_len = pointer_len_from_operand(tcx, body, state, &args[1].node, 8);
    let count = eval_call_arg(tcx, body, state, &args[2].node);
    let src_level = check_side(count, src_len);
    let dst_level = check_side(count, dst_len);
    let level = src_level.combine(dst_level);
    if level == CheckLevel::Safe {
        return;
    }

    let code = level.oob_code();
    let message = match level {
        CheckLevel::Definite => {
            "calling `ptr::copy_nonoverlapping` with a definitely out-of-bounds range"
        }
        CheckLevel::Possible => {
            "calling `ptr::copy_nonoverlapping` with a range that may exceed the source or destination object"
        }
        CheckLevel::Safe => unreachable!(),
    };

    let src_note = src_len
        .map(|len| format!("src_len = {len}"))
        .unwrap_or_else(|| "src_len = unknown".to_string());
    let dst_note = dst_len
        .map(|len| format!("dst_len = {len}"))
        .unwrap_or_else(|| "dst_len = unknown".to_string());
    emit_warning(
        tcx,
        term.source_info.span,
        code,
        message,
        &[format!("count = {count}"), src_note, dst_note],
    );
}
