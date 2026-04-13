use rustc_middle::mir::{Body, Operand};
use rustc_middle::ty::TyCtxt;
use rustc_span::Span;

use crate::nullptr::abstract_value::NullPtr;
use crate::nullptr::transfer::eval_operand;
use crate::nullptr::NullPtrState;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum WarningLevel {
    Safe,
    Possible,
    Definite,
}

pub(crate) fn level_for_value(value: NullPtr, warn_on_maybe: bool) -> WarningLevel {
    match value {
        NullPtr::NonNull => WarningLevel::Safe,
        NullPtr::Null => WarningLevel::Definite,
        NullPtr::MaybeNull | NullPtr::Bot => {
            if warn_on_maybe {
                WarningLevel::Possible
            } else {
                WarningLevel::Safe
            }
        }
    }
}

pub(crate) fn eval_call_arg<'tcx>(
    tcx: TyCtxt<'tcx>,
    body: &'tcx Body<'tcx>,
    state: &NullPtrState<'tcx>,
    arg: &Operand<'tcx>,
) -> NullPtr {
    let arg_ty = arg.ty(&body.local_decls, tcx);
    eval_operand(tcx, &body.local_decls, arg, state, arg_ty)
}

pub(crate) fn emit_warning<'tcx>(
    tcx: TyCtxt<'tcx>,
    span: Span,
    code: &str,
    message: &str,
    notes: &[String],
) {
    let sm = tcx.sess.source_map();
    let loc = sm.span_to_diagnostic_string(span);
    let snippet = sm
        .span_to_snippet(span)
        .unwrap_or_else(|_| "unsafe call".to_string())
        .replace('\n', " ");
    println!("warning[{code}]: {message}");
    println!("  --> {loc}");
    println!("   |");
    println!("   | {snippet}");
    println!("   | ^ unsafe call here");
    for note in notes {
        println!("   = {note}");
    }
}
