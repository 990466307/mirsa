use rustc_middle::mir::{Body, Terminator, TerminatorKind};
use rustc_middle::ty::TyCtxt;

use crate::finding::{Finding, Level};
use mirsa_domains::interval::IntervalState;

use super::shared::eval_call_arg;

pub(crate) fn check<'tcx>(
    tcx: TyCtxt<'tcx>,
    body: &Body<'tcx>,
    term: &Terminator<'tcx>,
    state: &IntervalState<'tcx>,
) -> Option<Finding> {
    let TerminatorKind::Call { args, .. } = &term.kind else {
        return None;
    };
    let Some(arg) = args.first() else {
        return None;
    };
    let value = eval_call_arg(tcx, body, state, &arg.node);
    let level = if value.is_empty() || value.low > 0 || value.high < 0 {
        Level::Safe
    } else if value.low == 0 && value.high == 0 {
        Level::Definite
    } else {
        Level::Possible
    };
    Finding::for_level(
        level,
        term.source_info.span,
        "interval/definite-zero",
        "interval/possible-zero",
        "calling `NonZero::new_unchecked` with an argument that is exactly 0",
        "calling `NonZero::new_unchecked` with an argument that may be 0",
        vec![format!("argument = {value}")],
    )
}
