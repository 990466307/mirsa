use crate::framework::access_path::AccessPath;
use crate::framework::forward::DomainState;
use crate::framework::printer::StateEntries;
use crate::framework::symbolic::{SymbolicState, join_display_places};
use rustc_middle::mir::Place;
use std::collections::HashMap;
use std::fmt;

use super::abstract_value::{Interval, join, widen};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IntervalState<'tcx> {
    interval: HashMap<AccessPath, Interval>,
    len: HashMap<AccessPath, Interval>,
    display_places: HashMap<AccessPath, Place<'tcx>>,
    debug: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IntervalAnalysisState<'tcx> {
    pub symbolic: SymbolicState<'tcx>,
    pub interval: IntervalState<'tcx>,
}

impl<'tcx> IntervalState<'tcx> {
    fn default(debug: bool) -> Self {
        IntervalState {
            interval: HashMap::new(),
            len: HashMap::new(),
            display_places: HashMap::new(),
            debug,
        }
    }

    pub fn new_bot_state(places: &[Place<'tcx>], arg_count: usize, debug: bool) -> Self {
        let mut interval = HashMap::new();
        let mut display_places = HashMap::new();

        for place in places {
            let Some(path) = Self::path_for_place(*place) else {
                continue;
            };
            let local_idx = place.local.index();
            let value = if local_idx >= 1 && local_idx <= arg_count {
                Interval::top()
            } else {
                Interval::empty()
            };
            interval.insert(path.clone(), value);
            display_places.insert(path, *place);
        }

        IntervalState {
            interval,
            len: HashMap::new(),
            display_places,
            debug,
        }
    }
}

impl<'tcx> DomainState<'tcx> for IntervalState<'tcx> {
    fn join(left: &Self, right: &Self) -> Self {
        join_state(left, right)
    }

    fn widen(previous: &Self, next: &Self) -> Self {
        widen_state(previous, next)
    }

    fn state_changed(previous: &Self, next: &Self) -> bool {
        previous.interval != next.interval || previous.len != next.len
    }
}

impl<'tcx> fmt::Display for IntervalState<'tcx> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut entries: Vec<(String, String)> = self
            .interval
            .iter()
            .map(|(path, interval)| (path.to_string(), interval.to_string()))
            .collect();
        entries.sort_by(|a, b| a.0.cmp(&b.0));

        for (idx, (place, interval)) in entries.iter().enumerate() {
            if idx > 0 {
                write!(f, ", ")?;
            }
            write!(f, "{place} => {interval}")?;
        }
        Ok(())
    }
}

impl<'tcx> StateEntries<'tcx> for IntervalState<'tcx> {
    fn entries(&self) -> Vec<(Place<'tcx>, String)> {
        self.interval
            .iter()
            .filter_map(|(path, interval)| {
                self.display_places
                    .get(path)
                    .map(|place| (*place, interval.to_string()))
            })
            .collect()
    }

    fn should_print_entry(&self, place: Place<'tcx>) -> bool {
        let Some(path) = Self::path_for_place(place) else {
            return false;
        };
        self.interval
            .get(&path)
            .is_some_and(|interval| !interval.is_empty())
    }
}

impl<'tcx> IntervalState<'tcx> {
    pub fn debug(&self, args: fmt::Arguments<'_>) {
        if self.debug {
            eprintln!("[interval] {args}");
        }
    }

    pub fn debug_map(&self, label: &str) {
        if !self.debug {
            return;
        }

        let mut entries: Vec<_> = self
            .interval
            .iter()
            .map(|(path, value)| (path.to_string(), value.to_string()))
            .collect();
        entries.sort_by(|a, b| a.0.cmp(&b.0));

        eprintln!("[interval] {label}:");
        if entries.is_empty() && self.len.is_empty() {
            eprintln!("[interval]   <empty>");
            return;
        }
        for (place, value) in entries {
            eprintln!("[interval]   {place} => {value}");
        }

        let mut len_entries: Vec<_> = self
            .len
            .iter()
            .map(|(path, value)| (format!("{path}.len"), value.to_string()))
            .collect();
        len_entries.sort_by(|a, b| a.0.cmp(&b.0));
        for (place, value) in len_entries {
            eprintln!("[interval]   {place} => {value}");
        }
    }

    pub fn path_for_place(place: Place<'tcx>) -> Option<AccessPath> {
        AccessPath::from_place(place)
    }

    pub fn get_interval(&self, place: &Place<'tcx>) -> Interval {
        let Some(path) = Self::path_for_place(*place) else {
            return Interval::empty();
        };
        self.interval
            .get(&path)
            .copied()
            .unwrap_or_else(Interval::empty)
    }

    pub fn set_interval(&mut self, place: Place<'tcx>, interval: Interval) {
        let Some(path) = Self::path_for_place(place) else {
            return;
        };
        let old = self.interval.insert(path.clone(), interval);
        if old != Some(interval) {
            self.debug(format_args!("{place:?} := {interval}"));
        }
        self.display_places.insert(path, place);
    }

    pub fn get_len(&self, place: &Place<'tcx>) -> Option<Interval> {
        let path = Self::path_for_place(*place)?;
        self.len.get(&path).copied()
    }

    pub fn set_len(&mut self, place: Place<'tcx>, len: Interval) {
        let Some(path) = Self::path_for_place(place) else {
            return;
        };
        let old = self.len.insert(path.clone(), len);
        if old != Some(len) {
            self.debug(format_args!("{place:?}.len := {len}"));
        }
        self.display_places.insert(path, place);
    }

    pub fn clear_len(&mut self, place: &Place<'tcx>) {
        let Some(path) = Self::path_for_place(*place) else {
            return;
        };
        if self.len.remove(&path).is_some() {
            self.debug(format_args!("clear {:?}.len", place));
        }
    }

    pub fn all_fact_places(&self) -> Vec<Place<'tcx>> {
        self.interval
            .keys()
            .chain(self.len.keys())
            .filter_map(|path| self.display_places.get(path).copied())
            .collect()
    }

    pub fn interval_places(&self) -> Vec<Place<'tcx>> {
        self.interval
            .keys()
            .filter_map(|path| self.display_places.get(path).copied())
            .collect()
    }

    pub fn merge_display_places_into(&self, symbolic: &mut SymbolicState<'tcx>) {
        symbolic.remember_places(
            self.display_places
                .iter()
                .map(|(path, place)| (path.clone(), *place)),
        );
    }
}

pub fn join_state<'tcx>(a: &IntervalState<'tcx>, b: &IntervalState<'tcx>) -> IntervalState<'tcx> {
    let mut out = IntervalState::default(a.debug || b.debug);
    for k in a.interval.keys().chain(b.interval.keys()) {
        let ia = a.interval.get(k).copied().unwrap_or_else(Interval::empty);
        let ib = b.interval.get(k).copied().unwrap_or_else(Interval::empty);
        out.interval.insert(k.clone(), join(&ia, &ib));
    }
    for k in a.len.keys().chain(b.len.keys()) {
        let ia = a.len.get(k).copied().unwrap_or_else(Interval::empty);
        let ib = b.len.get(k).copied().unwrap_or_else(Interval::empty);
        out.len.insert(k.clone(), join(&ia, &ib));
    }
    out.display_places = join_display_places(&a.display_places, &b.display_places);
    out
}

pub fn widen_state<'tcx>(a: &IntervalState<'tcx>, b: &IntervalState<'tcx>) -> IntervalState<'tcx> {
    let mut out = IntervalState::default(a.debug || b.debug);
    for k in a.interval.keys().chain(b.interval.keys()) {
        let ia = a.interval.get(k).copied().unwrap_or_else(Interval::empty);
        let ib = b.interval.get(k).copied().unwrap_or_else(Interval::empty);
        let widened = widen(&ia, &ib);
        out.interval.insert(k.clone(), widened);
    }
    for k in a.len.keys().chain(b.len.keys()) {
        let ia = a.len.get(k).copied().unwrap_or_else(Interval::empty);
        let ib = b.len.get(k).copied().unwrap_or_else(Interval::empty);
        out.len.insert(k.clone(), widen(&ia, &ib));
    }
    out.display_places = join_display_places(&a.display_places, &b.display_places);
    out
}

impl<'tcx> IntervalAnalysisState<'tcx> {
    pub fn new(interval: IntervalState<'tcx>) -> Self {
        let mut symbolic = SymbolicState::new();
        interval.merge_display_places_into(&mut symbolic);
        Self { symbolic, interval }
    }
}

impl<'tcx> DomainState<'tcx> for IntervalAnalysisState<'tcx> {
    fn join(left: &Self, right: &Self) -> Self {
        Self {
            symbolic: SymbolicState::join(&left.symbolic, &right.symbolic),
            interval: IntervalState::join(&left.interval, &right.interval),
        }
    }

    fn widen(previous: &Self, next: &Self) -> Self {
        Self {
            symbolic: SymbolicState::join(&previous.symbolic, &next.symbolic),
            interval: IntervalState::widen(&previous.interval, &next.interval),
        }
    }

    fn state_changed(previous: &Self, next: &Self) -> bool {
        IntervalState::state_changed(&previous.interval, &next.interval)
            || previous.symbolic != next.symbolic
    }
}

impl<'tcx> fmt::Display for IntervalAnalysisState<'tcx> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.interval.fmt(f)
    }
}

impl<'tcx> StateEntries<'tcx> for IntervalAnalysisState<'tcx> {
    fn entries(&self) -> Vec<(Place<'tcx>, String)> {
        self.interval.entries()
    }

    fn should_print_entry(&self, place: Place<'tcx>) -> bool {
        self.interval.should_print_entry(place)
    }
}
