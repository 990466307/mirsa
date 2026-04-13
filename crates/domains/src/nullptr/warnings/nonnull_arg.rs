use rustc_middle::mir::{Body, Terminator, TerminatorKind};
use rustc_middle::ty::TyCtxt;

use crate::nullptr::NullPtrState;

use super::shared::{WarningLevel, emit_warning, eval_call_arg, level_for_value};

pub(crate) fn matches_path(path: &str) -> bool {
    (path.ends_with("::new_unchecked") && path.contains("::NonNull"))
        || (path.ends_with("::from_ptr") && path.contains("::CStr"))
        || (path.ends_with("::from_raw_parts") && path.contains("::Vec"))
        || (path.ends_with("::read") && path.contains("::ptr::"))
        || (path.ends_with("::write") && path.contains("::ptr::"))
}

pub(crate) fn emit<'tcx>(
    tcx: TyCtxt<'tcx>,
    body: &'tcx Body<'tcx>,
    term: &Terminator<'tcx>,
    state: &NullPtrState<'tcx>,
    warn_on_maybe: bool,
) {
    let TerminatorKind::Call { args, .. } = &term.kind else {
        return;
    };
    let Some(first_arg) = args.first() else {
        return;
    };
    let value = eval_call_arg(tcx, body, state, &first_arg.node);
    let level = level_for_value(value, warn_on_maybe);
    if level == WarningLevel::Safe {
        return;
    }

    let (code, message) = match level {
        WarningLevel::Definite => (
            "nullptr/definite-null",
            "call definitely receives a null pointer argument",
        ),
        WarningLevel::Possible => (
            "nullptr/possible-null",
            "call may receive a null pointer argument",
        ),
        WarningLevel::Safe => unreachable!(),
    };
    emit_warning(
        tcx,
        term.source_info.span,
        code,
        message,
        &[format!("argument = {value}")],
    );
}
