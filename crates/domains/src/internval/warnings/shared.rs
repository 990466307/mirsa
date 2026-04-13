use rustc_middle::mir::{Body, Operand, Place};
use rustc_middle::ty::{TyCtxt, TyKind};
use rustc_span::Span;

use crate::internval::{Internval, InternvalState};

use super::super::transfer::eval_operand;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CheckLevel {
    Safe,
    Possible,
    Definite,
}

pub(crate) fn format_callsite<'tcx>(tcx: TyCtxt<'tcx>, span: Span) -> (String, String) {
    let sm = tcx.sess.source_map();
    let loc = sm.span_to_diagnostic_string(span);
    let snippet = sm
        .span_to_snippet(span)
        .unwrap_or_else(|_| "unsafe call".to_string())
        .replace('\n', " ");
    (loc, snippet)
}

pub(crate) fn emit_warning<'tcx>(
    tcx: TyCtxt<'tcx>,
    span: Span,
    code: &str,
    message: &str,
    notes: &[String],
) {
    let (loc, snippet) = format_callsite(tcx, span);
    println!("warning[{code}]: {message}");
    println!("  --> {loc}");
    println!("   |");
    println!("   | {snippet}");
    println!("   | ^ unsafe call here");
    for note in notes {
        println!("   = {note}");
    }
}

pub(crate) fn eval_call_arg<'tcx>(
    tcx: TyCtxt<'tcx>,
    body: &'tcx Body<'tcx>,
    state: &InternvalState<'tcx>,
    arg: &Operand<'tcx>,
) -> Internval {
    eval_operand(tcx, &body.local_decls, arg, state)
}

pub(crate) fn operand_len<'tcx>(
    tcx: TyCtxt<'tcx>,
    body: &'tcx Body<'tcx>,
    state: &InternvalState<'tcx>,
    op: &Operand<'tcx>,
) -> Internval {
    match op {
        Operand::Copy(place) | Operand::Move(place) => place_len(tcx, body, state, *place),
        Operand::Constant(_) => Internval::top(),
    }
}

pub(crate) fn place_len<'tcx>(
    tcx: TyCtxt<'tcx>,
    body: &'tcx Body<'tcx>,
    state: &InternvalState<'tcx>,
    place: Place<'tcx>,
) -> Internval {
    let ty = place.ty(&body.local_decls, tcx).ty;
    match ty.kind() {
        TyKind::Array(_, len) => len
            .try_to_target_usize(tcx)
            .map(|len| Internval::new(len as i128, len as i128))
            .unwrap_or_else(Internval::top),
        TyKind::Slice(_) => state.get_slice_meta(&place).unwrap_or_else(Internval::top),
        TyKind::Ref(_, inner, _) => match inner.kind() {
            TyKind::Array(_, len) => len
                .try_to_target_usize(tcx)
                .map(|len| Internval::new(len as i128, len as i128))
                .unwrap_or_else(Internval::top),
            TyKind::Slice(_) => state.get_slice_meta(&place).unwrap_or_else(Internval::top),
            _ => Internval::top(),
        },
        _ => Internval::top(),
    }
}

pub(crate) fn check_lt(index: Internval, len: Internval) -> CheckLevel {
    if index.is_empty() || len.is_empty() {
        return CheckLevel::Safe;
    }
    if index.high < 0 {
        return CheckLevel::Definite;
    }
    if index.low >= 0 && index.high < len.low {
        return CheckLevel::Safe;
    }
    if index.low >= len.high {
        return CheckLevel::Definite;
    }
    CheckLevel::Possible
}

pub(crate) fn check_le(index: Internval, len: Internval) -> CheckLevel {
    if index.is_empty() || len.is_empty() {
        return CheckLevel::Safe;
    }
    if index.high < 0 {
        return CheckLevel::Definite;
    }
    if index.low >= 0 && index.high <= len.low {
        return CheckLevel::Safe;
    }
    if index.low > len.high {
        return CheckLevel::Definite;
    }
    CheckLevel::Possible
}
