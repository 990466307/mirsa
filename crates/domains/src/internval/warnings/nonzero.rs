use rustc_middle::mir::{Body, Terminator, TerminatorKind};
use rustc_middle::ty::TyCtxt;

use crate::internval::InternvalState;

use super::shared::{CheckLevel, emit_warning, eval_call_arg};

pub(crate) fn matches_path(path: &str) -> bool {
    path.ends_with("::new_unchecked") && path.contains("::NonZero")
}

pub(crate) fn emit<'tcx>(
    tcx: TyCtxt<'tcx>,
    body: &'tcx Body<'tcx>,
    term: &Terminator<'tcx>,
    state: &InternvalState<'tcx>,
) {
    let TerminatorKind::Call { args, .. } = &term.kind else {
        return;
    };
    let Some(arg) = args.first() else {
        return;
    };
    let value = eval_call_arg(tcx, body, state, &arg.node);
    let level = if value.is_empty() || value.low > 0 || value.high < 0 {
        CheckLevel::Safe
    } else if value.low == 0 && value.high == 0 {
        CheckLevel::Definite
    } else {
        CheckLevel::Possible
    };
    if level == CheckLevel::Safe {
        return;
    }

    let code = match level {
        CheckLevel::Definite => "internval/definite-zero",
        CheckLevel::Possible => "internval/possible-zero",
        CheckLevel::Safe => unreachable!(),
    };
    let message = match level {
        CheckLevel::Definite => {
            "calling `NonZero::new_unchecked` with an argument that is exactly 0"
        }
        CheckLevel::Possible => {
            "calling `NonZero::new_unchecked` with an argument that may be 0"
        }
        CheckLevel::Safe => unreachable!(),
    };
    emit_warning(
        tcx,
        term.source_info.span,
        code,
        message,
        &[format!("argument = {value}")],
    );
}
