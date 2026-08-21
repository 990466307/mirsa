use super::interval::{ResolvedPlaces, first_resolved_place, has_runtime_index, resolve_places};
use mirsa_domains::interval::IntervalState;
use mirsa_domains::nullptr::abstract_value::join as join_nullptr;
use mirsa_domains::nullptr::transfer::{eval_operand, is_tracked};
use mirsa_domains::nullptr::{NullPtr, NullPtrState};
use mirsa_framework::printer::StateEntries;
use mirsa_relations::symbolic::SymbolicState;
use rustc_middle::mir::{LocalDecls, Operand, Place, Rvalue, Statement, StatementKind};
use rustc_middle::ty::{Ty, TyCtxt, TyKind};

pub fn reduce_statement<'tcx>(
    tcx: TyCtxt<'tcx>,
    interval: &mut IntervalState<'tcx>,
    nullptr: &mut NullPtrState<'tcx>,
    symbolic: &SymbolicState<'tcx>,
    statement: &Statement<'tcx>,
    local_decls: &LocalDecls<'tcx>,
) {
    let StatementKind::Assign(assign) = &statement.kind else {
        return;
    };
    let (destination, rvalue) = &**assign;

    if has_runtime_index(*destination) {
        let places = resolve_places(tcx, local_decls, interval, symbolic, *destination);
        reduce_indexed_write(tcx, local_decls, nullptr, symbolic, &places, rvalue);
    }

    let Rvalue::Use(Operand::Copy(source) | Operand::Move(source)) = rvalue else {
        return;
    };
    if has_runtime_index(*source) && !has_runtime_index(*destination) {
        let places = resolve_places(tcx, local_decls, interval, symbolic, *source);
        reduce_indexed_read(tcx, local_decls, nullptr, symbolic, *destination, &places);
    }
}

/// Reduce the interval × nullptr product for facts represented in both
/// components.
pub fn reduce<'tcx>(
    interval: &IntervalState<'tcx>,
    nullptr: &mut NullPtrState<'tcx>,
    symbolic: &SymbolicState<'tcx>,
) {
    let tracked_places: Vec<_> = nullptr
        .entries()
        .into_iter()
        .map(|(place, _)| place)
        .collect();

    for place in tracked_places {
        let Some(path) = nullptr.access_path_for_place_resolved(symbolic, place) else {
            continue;
        };
        if nullptr.value_or_maybe(&path) != NullPtr::MaybeNull {
            continue;
        }
        let Some(value) = interval.tracked_interval_resolved(symbolic, &place) else {
            continue;
        };
        if value.is_empty() {
            continue;
        }

        let refined = if value.low == 0 && value.high == 0 {
            Some(NullPtr::Null)
        } else if value.high < 0 || value.low > 0 {
            Some(NullPtr::NonNull)
        } else {
            None
        };
        if let Some(refined) = refined {
            nullptr.debug(format_args!(
                "reduce from interval {path}: {value} -> {refined}"
            ));
            nullptr.set_path(path, refined);
        }
    }
}

fn reduce_indexed_read<'tcx>(
    tcx: TyCtxt<'tcx>,
    local_decls: &LocalDecls<'tcx>,
    state: &mut NullPtrState<'tcx>,
    symbolic: &SymbolicState<'tcx>,
    destination: Place<'tcx>,
    places: &ResolvedPlaces<'tcx>,
) {
    if !is_tracked(destination.ty(local_decls, tcx).ty) {
        return;
    }
    let value = read_nullptr(state, symbolic, places);
    state.debug(format_args!(
        "reduce indexed read {:?} := {}",
        destination, value
    ));
    state.set_place_path_resolved(symbolic, destination, value);
}

fn reduce_indexed_write<'tcx>(
    tcx: TyCtxt<'tcx>,
    local_decls: &LocalDecls<'tcx>,
    state: &mut NullPtrState<'tcx>,
    symbolic: &SymbolicState<'tcx>,
    places: &ResolvedPlaces<'tcx>,
    rvalue: &Rvalue<'tcx>,
) {
    let Some(place) = first_resolved_place(places) else {
        return;
    };
    let ty = place.ty(local_decls, tcx).ty;
    if !is_tracked(ty) {
        return;
    }
    let value = eval_nullptr_rvalue(tcx, local_decls, state, symbolic, ty, rvalue);
    match places {
        ResolvedPlaces::Exact(place) => {
            state.debug(format_args!(
                "reduce indexed write {:?} := {}",
                place, value
            ));
            state.set_place_path_resolved(symbolic, *place, value);
        }
        ResolvedPlaces::Candidates(places) => {
            for place in places {
                let Some(path) = symbolic.normalize_place(*place) else {
                    continue;
                };
                state.debug(format_args!("reduce indexed weak write {path} := {value}"));
                state.join_path(path, value);
            }
        }
        ResolvedPlaces::Summary(place) => {
            let Some(path) = symbolic.normalize_place(*place) else {
                return;
            };
            state.debug(format_args!(
                "reduce indexed summary write {path} := {value}"
            ));
            state.join_path(path, value);
        }
    }
}

fn read_nullptr<'tcx>(
    state: &NullPtrState<'tcx>,
    symbolic: &SymbolicState<'tcx>,
    places: &ResolvedPlaces<'tcx>,
) -> NullPtr {
    match places {
        ResolvedPlaces::Exact(place) => symbolic
            .normalize_place(*place)
            .map(|path| state.value_or_maybe(&path))
            .unwrap_or(NullPtr::MaybeNull),
        ResolvedPlaces::Candidates(places) => places
            .iter()
            .filter_map(|place| symbolic.normalize_place(*place))
            .map(|path| state.value_or_maybe(&path))
            .reduce(join_nullptr)
            .unwrap_or(NullPtr::MaybeNull),
        ResolvedPlaces::Summary(place) => {
            let Some(pattern) = symbolic.normalize_place(*place) else {
                return NullPtr::MaybeNull;
            };
            state
                .fact_paths()
                .filter(|path| path.matches_pattern(&pattern) || *path == pattern)
                .map(|path| state.value_or_maybe(&path))
                .reduce(join_nullptr)
                .unwrap_or(NullPtr::MaybeNull)
        }
    }
}

fn eval_nullptr_rvalue<'tcx>(
    tcx: TyCtxt<'tcx>,
    local_decls: &LocalDecls<'tcx>,
    state: &mut NullPtrState<'tcx>,
    symbolic: &SymbolicState<'tcx>,
    destination_ty: Ty<'tcx>,
    rvalue: &Rvalue<'tcx>,
) -> NullPtr {
    match rvalue {
        Rvalue::Use(operand) | Rvalue::Cast(_, operand, _) => {
            eval_operand(tcx, local_decls, operand, state, symbolic, destination_ty)
        }
        Rvalue::Ref(..) => NullPtr::NonNull,
        Rvalue::RawPtr(..) => NullPtr::MaybeNull,
        _ if matches!(destination_ty.kind(), TyKind::Ref(_, _, _)) => NullPtr::NonNull,
        _ => NullPtr::MaybeNull,
    }
}
