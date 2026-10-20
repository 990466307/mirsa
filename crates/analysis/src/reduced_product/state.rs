use mirsa_domains::allocation::AllocationState;
use mirsa_domains::interval::IntervalState;
use mirsa_domains::nullptr::NullPtrState;
use mirsa_framework::access_path::AccessPath;
use mirsa_framework::forward::DomainState;
use mirsa_framework::printer::StateEntries;
use mirsa_relations::symbolic::SymbolicState;
use rustc_middle::mir::Place;
use std::collections::HashSet;
use std::fmt;

#[derive(Clone, Debug)]
pub struct CombinedState<'tcx> {
    pub symbolic: SymbolicState<'tcx>,
    pub interval: IntervalState<'tcx>,
    pub nullptr: NullPtrState<'tcx>,
    pub allocation: AllocationState<'tcx>,
    pub reduce_debug: bool,
    pub(crate) feasible: bool,
    pub(crate) writes: Option<HashSet<AccessPath>>,
    pub(super) reduction_memo: super::memo::ReductionMemo<'tcx>,
}

impl<'tcx> CombinedState<'tcx> {
    pub fn new(interval: IntervalState<'tcx>, nullptr: NullPtrState<'tcx>) -> Self {
        Self::new_with_debug(
            interval,
            nullptr,
            AllocationState::bottom(false),
            false,
            false,
        )
    }

    pub fn new_with_debug(
        interval: IntervalState<'tcx>,
        nullptr: NullPtrState<'tcx>,
        allocation: AllocationState<'tcx>,
        symbolic_debug: bool,
        reduce_debug: bool,
    ) -> Self {
        let mut symbolic = SymbolicState::new_with_debug(symbolic_debug);
        interval.merge_display_places_into(&mut symbolic);
        nullptr.merge_display_places_into(&mut symbolic);
        allocation.merge_display_places_into(&mut symbolic);
        Self {
            symbolic,
            interval,
            nullptr,
            allocation,
            reduce_debug,
            feasible: true,
            writes: None,
            reduction_memo: Default::default(),
        }
    }

    pub fn from_parts(
        symbolic: SymbolicState<'tcx>,
        interval: IntervalState<'tcx>,
        nullptr: NullPtrState<'tcx>,
        allocation: AllocationState<'tcx>,
        reduce_debug: bool,
    ) -> Self {
        Self {
            symbolic,
            interval,
            nullptr,
            allocation,
            reduce_debug,
            feasible: true,
            writes: None,
            reduction_memo: Default::default(),
        }
    }

    pub fn debug_reduce(&self, args: fmt::Arguments<'_>) {
        if self.reduce_debug {
            eprintln!("[reduce] {args}");
        }
    }

    pub(crate) fn record_writes(&mut self, paths: impl IntoIterator<Item = AccessPath>) {
        if let Some(writes) = &mut self.writes {
            writes.extend(paths);
        }
    }

    pub(crate) fn extend(&mut self, other: Self) {
        self.symbolic.extend(other.symbolic);
        self.interval.extend(other.interval);
        self.nullptr.extend(other.nullptr);
        self.allocation.extend(other.allocation);
    }
}

impl<'tcx> DomainState<'tcx> for CombinedState<'tcx> {
    fn join(left: &Self, right: &Self) -> Self {
        if !left.feasible {
            return right.clone();
        }
        if !right.feasible {
            return left.clone();
        }
        Self {
            symbolic: SymbolicState::join(&left.symbolic, &right.symbolic),
            interval: IntervalState::join(&left.interval, &right.interval),
            nullptr: NullPtrState::join(&left.nullptr, &right.nullptr),
            allocation: AllocationState::join(&left.allocation, &right.allocation),
            reduce_debug: left.reduce_debug || right.reduce_debug,
            feasible: true,
            writes: merge_writes(left, right),
            reduction_memo: Default::default(),
        }
    }

    fn widen(previous: &Self, next: &Self) -> Self {
        if !previous.feasible {
            return next.clone();
        }
        if !next.feasible {
            return previous.clone();
        }
        Self {
            symbolic: SymbolicState::join(&previous.symbolic, &next.symbolic),
            interval: IntervalState::widen(&previous.interval, &next.interval),
            nullptr: NullPtrState::widen(&previous.nullptr, &next.nullptr),
            allocation: AllocationState::widen(&previous.allocation, &next.allocation),
            reduce_debug: previous.reduce_debug || next.reduce_debug,
            feasible: true,
            writes: merge_writes(previous, next),
            reduction_memo: Default::default(),
        }
    }

    fn state_changed(previous: &Self, next: &Self) -> bool {
        previous.feasible != next.feasible
            || previous.writes != next.writes
            || IntervalState::state_changed(&previous.interval, &next.interval)
            || NullPtrState::state_changed(&previous.nullptr, &next.nullptr)
            || AllocationState::state_changed(&previous.allocation, &next.allocation)
            || previous.symbolic != next.symbolic
            || previous.reduce_debug != next.reduce_debug
    }
}

fn merge_writes(
    left: &CombinedState<'_>,
    right: &CombinedState<'_>,
) -> Option<HashSet<AccessPath>> {
    match (&left.writes, &right.writes) {
        (None, None) => None,
        (left, right) => Some(
            left.iter()
                .chain(right.iter())
                .flat_map(|paths| paths.iter().cloned())
                .collect(),
        ),
    }
}

impl<'tcx> fmt::Display for CombinedState<'tcx> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "interval: {{{}}}, nullptr: {{{}}}, allocation: {{{}}}",
            self.interval, self.nullptr, self.allocation
        )
    }
}

impl<'tcx> StateEntries<'tcx> for CombinedState<'tcx> {
    fn entries(&self) -> Vec<(Place<'tcx>, String)> {
        let mut entries = Vec::new();
        entries.extend(
            self.interval
                .entries()
                .into_iter()
                .filter(|(place, _)| self.interval.should_print_entry(*place))
                .map(|(place, value)| (place, format!("interval {value}"))),
        );
        entries.extend(
            self.nullptr
                .entries()
                .into_iter()
                .filter(|(place, _)| self.nullptr.should_print_entry(*place))
                .map(|(place, value)| (place, format!("nullptr {value}"))),
        );
        entries.extend(
            self.allocation
                .entries()
                .into_iter()
                .filter(|(place, _)| self.allocation.should_print_entry(*place))
                .map(|(place, value)| (place, format!("allocation {value}"))),
        );
        entries
    }

    fn should_print_entry(&self, place: Place<'tcx>) -> bool {
        self.interval.should_print_entry(place)
            || self.nullptr.should_print_entry(place)
            || self.allocation.should_print_entry(place)
    }
}

// Cache history must not participate in abstract-state equality or convergence.
impl PartialEq for CombinedState<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.symbolic == other.symbolic
            && self.interval == other.interval
            && self.nullptr == other.nullptr
            && self.allocation == other.allocation
            && self.reduce_debug == other.reduce_debug
            && self.feasible == other.feasible
            && self.writes == other.writes
    }
}
impl Eq for CombinedState<'_> {}
