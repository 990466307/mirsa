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
    if args.len() < 2 {
        return None;
    }

    let src = eval_call_arg(tcx, body, state, &args[0].node);
    let dst = eval_call_arg(tcx, body, state, &args[1].node);
    let src_level = level_for_value(src, warn_on_maybe);
    let dst_level = level_for_value(dst, warn_on_maybe);
    let level = src_level.combine(dst_level);
    Finding::for_level(
        level,
        term.source_info.span,
        "nullptr/definite-null",
        "nullptr/possible-null",
        "call definitely passes a null source or destination pointer",
        "call may pass a null source or destination pointer",
        vec![format!("src = {src}"), format!("dst = {dst}")],
    )
}
