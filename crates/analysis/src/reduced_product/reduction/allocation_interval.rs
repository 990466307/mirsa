use super::interval::{ResolvedPlaces, first_resolved_place, has_runtime_index, resolve_places};
use mirsa_domains::allocation::AllocationState;
use mirsa_domains::allocation::PointerValue;
use mirsa_domains::allocation::state::is_allocation_pointer;
use mirsa_domains::allocation::transfer::{
    eval_pointer_rvalue, layout_scalar_call_result, pointer_difference_call_result,
};
use mirsa_domains::interval::IntervalState;
use mirsa_domains::interval::transfer::eval_operand as eval_interval_operand;
use mirsa_relations::symbolic::SymbolicState;
use rustc_middle::mir::{
    LocalDecls, Operand, Place, Rvalue, Statement, StatementKind, Terminator, TerminatorKind,
};
use rustc_middle::ty::TyCtxt;

pub fn reduce_statement<'tcx>(
    tcx: TyCtxt<'tcx>,
    allocation: &mut AllocationState<'tcx>,
    interval: &mut IntervalState<'tcx>,
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
        reduce_indexed_write(
            tcx,
            local_decls,
            allocation,
            interval,
            symbolic,
            &places,
            rvalue,
        );
    }

    let Rvalue::Use(Operand::Copy(source) | Operand::Move(source)) = rvalue else {
        return;
    };
    if has_runtime_index(*source) && !has_runtime_index(*destination) {
        let places = resolve_places(tcx, local_decls, interval, symbolic, *source);
        reduce_indexed_read(
            tcx,
            local_decls,
            allocation,
            symbolic,
            *destination,
            &places,
        );
    }
}

/// Materialize a scalar projection of a tracked Layout in the interval domain.
pub fn reduce_terminator<'tcx>(
    tcx: TyCtxt<'tcx>,
    local_decls: &LocalDecls<'tcx>,
    interval: &mut IntervalState<'tcx>,
    allocation: &AllocationState<'tcx>,
    symbolic: &SymbolicState<'tcx>,
    term: &Terminator<'tcx>,
) {
    let TerminatorKind::Call { destination, .. } = &term.kind else {
        return;
    };
    let value = layout_scalar_call_result(tcx, symbolic, allocation, term, local_decls)
        .or_else(|| pointer_difference_call_result(tcx, symbolic, allocation, term, local_decls));
    let Some(value) = value else {
        return;
    };
    interval.debug(format_args!(
        "reduce scalar from allocation {:?}: {}",
        destination, value
    ));
    interval.set_interval_resolved(symbolic, *destination, value);
}

fn reduce_indexed_read<'tcx>(
    tcx: TyCtxt<'tcx>,
    local_decls: &LocalDecls<'tcx>,
    state: &mut AllocationState<'tcx>,
    symbolic: &SymbolicState<'tcx>,
    destination: Place<'tcx>,
    places: &ResolvedPlaces<'tcx>,
) {
    if !is_allocation_pointer(tcx, destination.ty(local_decls, tcx).ty) {
        return;
    }
    let value = read_pointer(state, symbolic, places);
    state.debug(format_args!(
        "reduce indexed read {:?} := {}",
        destination, value
    ));
    state.set_pointer_resolved(symbolic, destination, value);
}

fn reduce_indexed_write<'tcx>(
    tcx: TyCtxt<'tcx>,
    local_decls: &LocalDecls<'tcx>,
    state: &mut AllocationState<'tcx>,
    interval: &mut IntervalState<'tcx>,
    symbolic: &SymbolicState<'tcx>,
    places: &ResolvedPlaces<'tcx>,
    rvalue: &Rvalue<'tcx>,
) {
    let Some(place) = first_resolved_place(places) else {
        return;
    };
    let destination_ty = place.ty(local_decls, tcx).ty;
    if !is_allocation_pointer(tcx, destination_ty) {
        return;
    }
    let mut integer_value = |operand: &Operand<'tcx>| {
        eval_interval_operand(tcx, local_decls, operand, symbolic, interval)
    };
    let value = eval_pointer_rvalue(
        tcx,
        symbolic,
        state,
        rvalue,
        local_decls,
        destination_ty,
        &mut integer_value,
    );
    write_pointer(state, symbolic, places, value);
}

fn read_pointer<'tcx>(
    state: &AllocationState<'tcx>,
    symbolic: &SymbolicState<'tcx>,
    places: &ResolvedPlaces<'tcx>,
) -> PointerValue {
    match places {
        ResolvedPlaces::Exact(place) => state.pointer_value_resolved(symbolic, *place),
        ResolvedPlaces::Candidates(places) => places
            .iter()
            .copied()
            .map(|place| state.pointer_value_resolved(symbolic, place))
            .fold(PointerValue::bottom(), |left, right| {
                PointerValue::join(&left, &right)
            }),
        ResolvedPlaces::Summary(place) => state.pointer_value_resolved(symbolic, *place),
    }
}

fn write_pointer<'tcx>(
    state: &mut AllocationState<'tcx>,
    symbolic: &SymbolicState<'tcx>,
    places: &ResolvedPlaces<'tcx>,
    value: PointerValue,
) {
    match places {
        ResolvedPlaces::Exact(place) => {
            state.debug(format_args!(
                "reduce indexed write {:?} := {}",
                place, value
            ));
            state.set_pointer_resolved(symbolic, *place, value);
        }
        ResolvedPlaces::Candidates(places) => {
            for place in places {
                state.debug(format_args!(
                    "reduce indexed weak write {:?} := {}",
                    place, value
                ));
                state.join_pointer_resolved(symbolic, *place, value.clone());
            }
        }
        ResolvedPlaces::Summary(place) => {
            state.debug(format_args!(
                "reduce indexed summary write {:?} := {}",
                place, value
            ));
            state.join_pointer_resolved(symbolic, *place, value);
        }
    }
}
