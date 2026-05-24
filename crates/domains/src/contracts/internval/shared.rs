use rustc_middle::mir::{Body, Operand};
use rustc_middle::ty::TyCtxt;

use crate::internval::{Internval, InternvalState};

use crate::contracts::finding::Level;
use crate::internval::transfer::eval_operand;

pub(crate) fn eval_call_arg<'tcx>(
    tcx: TyCtxt<'tcx>,
    body: &Body<'tcx>,
    state: &InternvalState<'tcx>,
    arg: &Operand<'tcx>,
) -> Internval {
    eval_operand(tcx, &body.local_decls, arg, state)
}

pub(crate) fn check_lt(index: Internval, len: Internval) -> Level {
    if index.is_empty() || len.is_empty() {
        return Level::Safe;
    }
    if index.high < 0 {
        return Level::Definite;
    }
    if index.low >= 0 && index.high < len.low {
        return Level::Safe;
    }
    if index.low >= len.high {
        return Level::Definite;
    }
    Level::Possible
}

pub(crate) fn check_le(index: Internval, len: Internval) -> Level {
    if index.is_empty() || len.is_empty() {
        return Level::Safe;
    }
    if index.high < 0 {
        return Level::Definite;
    }
    if index.low >= 0 && index.high <= len.low {
        return Level::Safe;
    }
    if index.low > len.high {
        return Level::Definite;
    }
    Level::Possible
}
