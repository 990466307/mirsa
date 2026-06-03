use rustc_middle::mir::{Body, Operand};
use rustc_middle::ty::TyCtxt;

use crate::finding::Level;
use mirsa_domains::nullptr::NullPtrState;
use mirsa_domains::nullptr::abstract_value::NullPtr;
use mirsa_domains::nullptr::transfer::eval_operand;
use mirsa_relations::symbolic::SymbolicState;

pub(crate) fn level_for_value(value: NullPtr, warn_on_maybe: bool) -> Level {
    match value {
        NullPtr::NonNull => Level::Safe,
        NullPtr::Null => Level::Definite,
        NullPtr::MaybeNull | NullPtr::Bot => {
            if warn_on_maybe {
                Level::Possible
            } else {
                Level::Safe
            }
        }
    }
}

pub(crate) fn eval_call_arg<'tcx>(
    tcx: TyCtxt<'tcx>,
    body: &Body<'tcx>,
    state: &NullPtrState<'tcx>,
    arg: &Operand<'tcx>,
) -> NullPtr {
    let arg_ty = arg.ty(&body.local_decls, tcx);
    let symbolic = SymbolicState::new();
    eval_operand(
        tcx,
        &body.local_decls,
        arg,
        &mut state.clone(),
        &symbolic,
        arg_ty,
    )
}
