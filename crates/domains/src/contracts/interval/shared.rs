use rustc_middle::mir::{Body, Operand};
use rustc_middle::ty::TyCtxt;

use crate::interval::{Interval, IntervalState};

use crate::contracts::finding::Level;
use crate::interval::transfer::eval_operand;

pub(crate) fn eval_call_arg<'tcx>(
    tcx: TyCtxt<'tcx>,
    body: &Body<'tcx>,
    state: &IntervalState<'tcx>,
    arg: &Operand<'tcx>,
) -> Interval {
    eval_operand(tcx, &body.local_decls, arg, state)
}

pub(crate) fn check_lt(index: Interval, len: Interval) -> Level {
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

pub(crate) fn check_le(index: Interval, len: Interval) -> Level {
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
