use rustc_middle::mir::{Body, Terminator, TerminatorKind};
use rustc_middle::ty::TyCtxt;

use crate::finding::Finding;
use mirsa_domains::nullptr::NullPtrState;

use super::shared::{eval_call_arg, level_for_value};

pub(crate) fn check<'tcx>(
    tcx: TyCtxt<'tcx>,
    body: &Body<'tcx>,
    term: &Terminator<'tcx>,
    state: &NullPtrState<'tcx>,
    warn_on_maybe: bool,
) -> Option<Finding> {
    let TerminatorKind::Call { args, .. } = &term.kind else {
        return None;
    };
    let Some(first_arg) = args.first() else {
        return None;
    };
    let value = eval_call_arg(tcx, body, state, &first_arg.node);
    let level = level_for_value(value, warn_on_maybe);
    Finding::for_level(
        level,
        term.source_info.span,
        "nullptr/definite-null",
        "nullptr/possible-null",
        "call definitely receives a null pointer argument",
        "call may receive a null pointer argument",
        vec![format!("argument = {value}")],
    )
}
