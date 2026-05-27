use crate::framework::forward::DomainState;
use crate::framework::printer::StateEntries;
use crate::framework::symbolic::SymbolicState;
use crate::interval::IntervalState;
use crate::nullptr::NullPtrState;
use rustc_middle::mir::Place;
use std::fmt;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CombinedState<'tcx> {
    pub symbolic: SymbolicState<'tcx>,
    pub interval: IntervalState<'tcx>,
    pub nullptr: NullPtrState<'tcx>,
}

impl<'tcx> CombinedState<'tcx> {
    pub fn new(interval: IntervalState<'tcx>, nullptr: NullPtrState<'tcx>) -> Self {
        let mut symbolic = SymbolicState::new();
        interval.merge_display_places_into(&mut symbolic);
        nullptr.merge_display_places_into(&mut symbolic);
        Self {
            symbolic,
            interval,
            nullptr,
        }
    }
}

impl<'tcx> DomainState<'tcx> for CombinedState<'tcx> {
    fn join(left: &Self, right: &Self) -> Self {
        let mut out = Self {
            symbolic: SymbolicState::join(&left.symbolic, &right.symbolic),
            interval: IntervalState::join(&left.interval, &right.interval),
            nullptr: NullPtrState::join(&left.nullptr, &right.nullptr),
        };
        out.reduce();
        out
    }

    fn widen(previous: &Self, next: &Self) -> Self {
        let mut out = Self {
            symbolic: SymbolicState::join(&previous.symbolic, &next.symbolic),
            interval: IntervalState::widen(&previous.interval, &next.interval),
            nullptr: NullPtrState::widen(&previous.nullptr, &next.nullptr),
        };
        out.reduce();
        out
    }

    fn state_changed(previous: &Self, next: &Self) -> bool {
        IntervalState::state_changed(&previous.interval, &next.interval)
            || NullPtrState::state_changed(&previous.nullptr, &next.nullptr)
            || previous.symbolic != next.symbolic
    }
}

impl<'tcx> fmt::Display for CombinedState<'tcx> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "interval: {{{}}}, nullptr: {{{}}}",
            self.interval, self.nullptr
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
        entries
    }

    fn should_print_entry(&self, place: Place<'tcx>) -> bool {
        self.interval.should_print_entry(place) || self.nullptr.should_print_entry(place)
    }
}
