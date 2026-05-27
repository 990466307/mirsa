use rustc_middle::mir::*;
use rustc_middle::ty::{TyCtxt, TyKind};

use super::state::CombinedState;
use crate::interval::transfer::interval_of_const;

fn record_switch_fact<'tcx>(
    out: &mut CombinedState<'tcx>,
    discr: Operand<'tcx>,
    values_for_succ: &[u128],
    all_values: &[u128],
    is_otherwise: bool,
) {
    if is_otherwise {
        for value in all_values {
            out.symbolic.assume_ne_const(discr.clone(), *value);
        }
    } else if values_for_succ.len() == 1 {
        out.symbolic.assume_eq_const(discr, values_for_succ[0]);
    }
}

pub fn refine_edge<'tcx>(
    tcx: TyCtxt<'tcx>,
    body: &Body<'tcx>,
    pred: BasicBlock,
    succ: BasicBlock,
    in_state: &CombinedState<'tcx>,
) -> Option<CombinedState<'tcx>> {
    let term = body.basic_blocks[pred].terminator.as_ref()?;
    in_state.interval.debug(format_args!(
        "refine edge bb{} -> bb{} by {:?}",
        pred.index(),
        succ.index(),
        term.kind
    ));

    match &term.kind {
        TerminatorKind::Goto { target } => {
            if *target != succ {
                return None;
            }
            let mut out = in_state.clone();
            out.reduce_with_context(tcx, &body.local_decls)
                .then_some(out)
        }
        TerminatorKind::SwitchInt { discr, targets } => {
            let mut values_for_succ: Vec<u128> = Vec::new();
            let mut all_values: Vec<u128> = Vec::new();
            for (val, target) in targets.iter() {
                all_values.push(val);
                if target == succ {
                    values_for_succ.push(val);
                }
            }
            let is_otherwise = targets.otherwise() == succ;
            if values_for_succ.is_empty() && !is_otherwise {
                return None;
            }

            let mut out = in_state.clone();
            match discr {
                Operand::Copy(_) | Operand::Move(_) => {
                    record_switch_fact(
                        &mut out,
                        discr.clone(),
                        &values_for_succ,
                        &all_values,
                        is_otherwise,
                    );

                    let discr_ty = discr.ty(&body.local_decls, tcx);
                    if matches!(discr_ty.kind(), TyKind::Bool) {
                        if is_otherwise && all_values.contains(&0) && all_values.contains(&1) {
                            return None;
                        }
                    }

                    out.reduce_with_context(tcx, &body.local_decls)
                        .then_some(out)
                }
                Operand::Constant(c) => {
                    let c_iv = interval_of_const(c);
                    if c_iv.is_empty() || c_iv.low != c_iv.high {
                        return out
                            .reduce_with_context(tcx, &body.local_decls)
                            .then_some(out);
                    }
                    let c_val = c_iv.low;
                    let discr_ty = discr.ty(&body.local_decls, tcx);
                    let values_for_succ_i128: Vec<i128> = values_for_succ
                        .iter()
                        .filter_map(|value| {
                            crate::interval::transfer::switch_value_to_i128(tcx, discr_ty, *value)
                        })
                        .collect();
                    let all_values_i128: Vec<i128> = all_values
                        .iter()
                        .filter_map(|value| {
                            crate::interval::transfer::switch_value_to_i128(tcx, discr_ty, *value)
                        })
                        .collect();
                    let matches_succ_value = values_for_succ_i128.contains(&c_val);
                    let matches_any_value = all_values_i128.contains(&c_val);
                    if matches_succ_value || (is_otherwise && !matches_any_value) {
                        out.reduce_with_context(tcx, &body.local_decls)
                            .then_some(out)
                    } else {
                        None
                    }
                }
            }
        }
        _ => {
            let mut out = in_state.clone();
            out.reduce_with_context(tcx, &body.local_decls)
                .then_some(out)
        }
    }
}
