use rustc_middle::mir::{Body, Terminator, TerminatorKind};
use rustc_middle::ty::TyCtxt;

use crate::contracts::finding::Finding;
use crate::contracts::matcher::ContractCall;
use crate::internval::InternvalState;
use crate::internval::transfer::operand_len;

use super::shared::{check_le, check_lt, eval_call_arg};

pub(crate) fn check<'tcx>(
    tcx: TyCtxt<'tcx>,
    body: &Body<'tcx>,
    term: &Terminator<'tcx>,
    state: &InternvalState<'tcx>,
    call: ContractCall,
) -> Option<Finding> {
    let TerminatorKind::Call { args, .. } = &term.kind else {
        return None;
    };
    if args.len() < 2 {
        return None;
    }

    let receiver = &args[0].node;
    let index = eval_call_arg(tcx, body, state, &args[1].node);
    let len = operand_len(tcx, &body.local_decls, state, receiver);

    let (level, property) = match call {
        ContractCall::SliceSplitAtUnchecked | ContractCall::SliceSplitAtMutUnchecked => {
            (check_le(index, len), "`mid <= len`")
        }
        ContractCall::SliceGetUnchecked | ContractCall::SliceGetUncheckedMut => {
            (check_lt(index, len), "`index < len`")
        }
        _ => return None,
    };
    let api = match call {
        ContractCall::SliceGetUnchecked => "slice::get_unchecked",
        ContractCall::SliceGetUncheckedMut => "slice::get_unchecked_mut",
        ContractCall::SliceSplitAtUnchecked => "slice::split_at_unchecked",
        ContractCall::SliceSplitAtMutUnchecked => "slice::split_at_mut_unchecked",
        _ => return None,
    };
    Finding::for_level(
        level,
        term.source_info.span,
        "internval/definite-oob",
        "internval/possible-oob",
        format!("calling `{api}` with a definitely out-of-bounds argument"),
        format!("calling `{api}` with an argument that may violate {property}"),
        vec![format!("index = {index}"), format!("len = {len}")],
    )
}
