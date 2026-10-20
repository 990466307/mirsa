use super::value::{self, Eval, ValueType};
use super::{Comparison, Expr, Predicate};
use crate::reduced_product::CombinedState;
use mirsa_solver::{ScalarType, SolveResult, UnknownReason, Verdict};
use rustc_middle::{
    mir::Body,
    ty::{Ty, TyCtxt, TyKind, TypingEnv},
};

pub(super) fn scalar_type<'tcx>(tcx: TyCtxt<'tcx>, ty: Ty<'tcx>) -> Option<ScalarType> {
    if ty.is_bool() {
        return Some(ScalarType::BOOL);
    }
    if !matches!(ty.kind(), TyKind::Int(_) | TyKind::Uint(_) | TyKind::Char) {
        return None;
    }
    let layout = tcx
        .layout_of(TypingEnv::fully_monomorphized().as_query_input(ty))
        .ok()?;
    Some(ScalarType {
        bits: layout.size.bits() as u32,
        signed: matches!(ty.kind(), TyKind::Int(_)),
    })
}

pub(super) fn expr_type<'tcx>(
    tcx: TyCtxt<'tcx>,
    body: &Body<'tcx>,
    expr: &Expr<'tcx>,
) -> Option<ScalarType> {
    match value::ty(tcx, body, expr) {
        ValueType::Rust(t) => scalar_type(tcx, t),
        ValueType::Math => None,
    }
}

pub(super) fn evaluate<'tcx>(
    tcx: TyCtxt<'tcx>,
    body: &Body<'tcx>,
    state: &CombinedState<'tcx>,
    property: &Predicate<'tcx>,
) -> Option<bool> {
    let eval = Eval { tcx, body, state };
    match property {
        Predicate::Value(e) => eval.eval(e).boolean(),
        Predicate::Compare(op, a, b) => {
            if value::ty(tcx, body, a) != value::ty(tcx, body, b) {
                return None;
            }
            if let Some(result) = eval.compare(*op, a, b) {
                return Some(result);
            }
            if let (Some(a), Some(b)) = (eval.scalar_place(a), eval.scalar_place(b)) {
                if !a
                    .projection
                    .iter()
                    .chain(b.projection.iter())
                    .any(|p| matches!(p, rustc_middle::mir::ProjectionElem::Index(_)))
                    && state.symbolic.equiv_places_readonly(a, b)
                {
                    return Some(matches!(
                        op,
                        Comparison::Eq | Comparison::Le | Comparison::Ge
                    ));
                }
            }
            None
        }
        Predicate::Not(p) => evaluate(tcx, body, state, p).map(|v| !v),
        Predicate::All(values) | Predicate::Any(values) => {
            let all = matches!(property, Predicate::All(_));
            let mut unknown = false;
            for p in values {
                match evaluate(tcx, body, state, p) {
                    Some(v) if v != all => return Some(v),
                    None => unknown = true,
                    _ => {}
                }
            }
            (!unknown).then_some(all)
        }
    }
}

pub(super) fn prove<'tcx>(
    tcx: TyCtxt<'tcx>,
    body: &Body<'tcx>,
    state: &CombinedState<'tcx>,
    property: &Predicate<'tcx>,
) -> SolveResult {
    if !state.feasible {
        return SolveResult::known(Verdict::Unreachable);
    }
    match evaluate(tcx, body, state, property) {
        Some(true) => SolveResult::known(Verdict::Proven),
        Some(false) => SolveResult::known(Verdict::Refuted),
        None => SolveResult::unknown(UnknownReason::InsufficientInformation),
    }
}
