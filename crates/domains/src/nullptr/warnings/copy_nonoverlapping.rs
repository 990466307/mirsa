use rustc_middle::mir::{Body, Terminator, TerminatorKind};
use rustc_middle::ty::TyCtxt;

use crate::nullptr::NullPtrState;

use super::shared::{WarningLevel, emit_warning, eval_call_arg, level_for_value};

pub(crate) fn matches_path(path: &str) -> bool {
    path.ends_with("::copy_nonoverlapping")
}

pub(crate) fn emit<'tcx>(
    tcx: TyCtxt<'tcx>,
    body: &Body<'tcx>,
    term: &Terminator<'tcx>,
    state: &NullPtrState<'tcx>,
    warn_on_maybe: bool,
) {
    let TerminatorKind::Call { args, .. } = &term.kind else {
        return;
    };
    if args.len() < 2 {
        return;
    }

    let src = eval_call_arg(tcx, body, state, &args[0].node);
    let dst = eval_call_arg(tcx, body, state, &args[1].node);
    let src_level = level_for_value(src, warn_on_maybe);
    let dst_level = level_for_value(dst, warn_on_maybe);
    let level = if src_level == WarningLevel::Definite || dst_level == WarningLevel::Definite {
        WarningLevel::Definite
    } else if src_level == WarningLevel::Possible || dst_level == WarningLevel::Possible {
        WarningLevel::Possible
    } else {
        WarningLevel::Safe
    };
    if level == WarningLevel::Safe {
        return;
    }

    let (code, message) = match level {
        WarningLevel::Definite => (
            "nullptr/definite-null",
            "call definitely passes a null source or destination pointer",
        ),
        WarningLevel::Possible => (
            "nullptr/possible-null",
            "call may pass a null source or destination pointer",
        ),
        WarningLevel::Safe => unreachable!(),
    };
    emit_warning(
        tcx,
        term.source_info.span,
        code,
        message,
        &[format!("src = {src}"), format!("dst = {dst}")],
    );
}
