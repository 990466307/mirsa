use mirsa_domains::allocation::AllocationState;
use mirsa_domains::nullptr::{NullPtr, NullPtrState};
use mirsa_framework::printer::StateEntries;
use mirsa_relations::symbolic::SymbolicState;

/// Exchange only the null alternative of an allocation pointer value. Object
/// identity, offset, extent and lifetime remain owned by AllocationState.
pub fn reduce<'tcx>(
    allocation: &mut AllocationState<'tcx>,
    nullptr: &mut NullPtrState<'tcx>,
    symbolic: &SymbolicState<'tcx>,
) {
    let places: Vec<_> = nullptr
        .entries()
        .into_iter()
        .map(|(place, _)| place)
        .collect();
    for place in places {
        let Some(allocation_path) = allocation.pointer_path_resolved(symbolic, place) else {
            continue;
        };
        let Some(nullptr_path) = nullptr.access_path_for_place_resolved(symbolic, place) else {
            continue;
        };
        let pointer = allocation.pointer_value(&allocation_path);
        let nullness = nullptr.value_or_maybe(&nullptr_path);
        let refined = match nullness {
            NullPtr::Null => pointer.only_null(),
            NullPtr::NonNull => pointer.without_null(),
            NullPtr::Bot | NullPtr::MaybeNull => pointer.clone(),
        };
        if refined != pointer {
            allocation.debug(format_args!(
                "reduce from nullptr {allocation_path}: {pointer} ∩ {nullness} = {refined}"
            ));
            allocation.set_pointer(allocation_path, refined.clone());
        }

        let allocation_nullness = if refined.is_null_only() {
            Some(NullPtr::Null)
        } else if refined.is_definitely_non_null() {
            Some(NullPtr::NonNull)
        } else {
            None
        };
        if let Some(value) = allocation_nullness {
            let current = nullptr.value_or_maybe(&nullptr_path);
            if current == NullPtr::MaybeNull || current == NullPtr::Bot {
                nullptr.debug(format_args!(
                    "reduce from allocation {nullptr_path}: {refined} -> {value}"
                ));
                nullptr.set_path(nullptr_path, value);
            }
        }
    }
}
