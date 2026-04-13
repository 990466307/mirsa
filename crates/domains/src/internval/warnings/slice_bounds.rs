use rustc_middle::mir::{Body, Terminator, TerminatorKind};
use rustc_middle::ty::TyCtxt;

use crate::internval::InternvalState;

use super::shared::{CheckLevel, check_le, check_lt, emit_warning, eval_call_arg, operand_len};

pub(crate) fn matches_path(path: &str) -> bool {
    path.ends_with("::get_unchecked")
        || path.ends_with("::get_unchecked_mut")
        || path.ends_with("::split_at_unchecked")
        || path.ends_with("::split_at_mut_unchecked")
}

pub(crate) fn emit<'tcx>(
    tcx: TyCtxt<'tcx>,
    body: &'tcx Body<'tcx>,
    term: &Terminator<'tcx>,
    state: &InternvalState<'tcx>,
    path: &str,
) {
    let TerminatorKind::Call { args, .. } = &term.kind else {
        return;
    };
    if args.len() < 2 {
        return;
    }

    let receiver = &args[0].node;
    let index = eval_call_arg(tcx, body, state, &args[1].node);
    let len = operand_len(tcx, body, state, receiver);

    let (level, property) = if path.ends_with("::split_at_unchecked")
        || path.ends_with("::split_at_mut_unchecked")
    {
        (check_le(index, len), "`mid <= len`")
    } else {
        (check_lt(index, len), "`index < len`")
    };
    if level == CheckLevel::Safe {
        return;
    }

    let code = match level {
        CheckLevel::Definite => "internval/definite-oob",
        CheckLevel::Possible => "internval/possible-oob",
        CheckLevel::Safe => unreachable!(),
    };
    let api = if path.ends_with("::get_unchecked_mut") {
        "slice::get_unchecked_mut"
    } else if path.ends_with("::get_unchecked") {
        "slice::get_unchecked"
    } else if path.ends_with("::split_at_mut_unchecked") {
        "slice::split_at_mut_unchecked"
    } else {
        "slice::split_at_unchecked"
    };
    let message = match level {
        CheckLevel::Definite => format!("calling `{api}` with a definitely out-of-bounds argument"),
        CheckLevel::Possible => format!("calling `{api}` with an argument that may violate {property}"),
        CheckLevel::Safe => unreachable!(),
    };
    emit_warning(
        tcx,
        term.source_info.span,
        code,
        &message,
        &[format!("index = {index}"), format!("len = {len}")],
    );
}
