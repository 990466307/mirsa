use crate::framework::eq_domain::{EqDomain, join_eq};
use crate::framework::forward::DomainState;
use crate::framework::printer::StateEntries;
use rustc_middle::mir::Place;
use std::collections::HashMap;
use std::fmt;

use super::abstract_value::{Internval, join, widen};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InternvalState<'tcx> {
    pub internval: HashMap<Place<'tcx>, Internval>,
    pub len: HashMap<Place<'tcx>, Internval>,
    pub eq: EqDomain<'tcx>,
}

impl<'tcx> InternvalState<'tcx> {
    fn default() -> Self {
        InternvalState {
            internval: HashMap::new(),
            len: HashMap::new(),
            eq: EqDomain::new(),
        }
    }

    pub fn new_bot_state(places: &[Place<'tcx>], arg_count: usize) -> Self {
        let mut internval = HashMap::new();
        let mut eq = EqDomain::new();

        for place in places {
            let local_idx = place.local.index();
            let value = if local_idx >= 1 && local_idx <= arg_count {
                Internval::top()
            } else {
                Internval::empty()
            };
            internval.insert(*place, value);
            eq.kill(*place);
        }

        InternvalState {
            internval,
            len: HashMap::new(),
            eq,
        }
    }
}

impl<'tcx> DomainState<'tcx> for InternvalState<'tcx> {
    fn join(left: &Self, right: &Self) -> Self {
        join_state(left, right)
    }

    fn widen(previous: &Self, next: &Self) -> Self {
        widen_state(previous, next)
    }

    fn state_changed(previous: &Self, next: &Self) -> bool {
        previous.internval != next.internval || previous.len != next.len
    }
}

impl<'tcx> fmt::Display for InternvalState<'tcx> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut entries: Vec<(String, String)> = self
            .internval
            .iter()
            .map(|(place, interval)| (format!("{place:?}"), interval.to_string()))
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

impl<'tcx> StateEntries<'tcx> for InternvalState<'tcx> {
    fn entries(&self) -> Vec<(Place<'tcx>, String)> {
        self.internval
            .iter()
            .map(|(place, interval)| (*place, interval.to_string()))
            .collect()
    }

    fn should_print_entry(&self, place: Place<'tcx>) -> bool {
        self.internval
            .get(&place)
            .is_some_and(|interval| !interval.is_empty())
    }
}

impl<'tcx> InternvalState<'tcx> {
    pub fn get_internval(&self, place: &Place<'tcx>) -> Internval {
        self.internval
            .get(place)
            .copied()
            .unwrap_or_else(Internval::empty)
    }
    pub fn set_internval(&mut self, place: Place<'tcx>, internval: Internval) {
        self.internval.insert(place, internval);
    }

    pub fn get_len(&self, place: &Place<'tcx>) -> Option<Internval> {
        self.len.get(place).copied()
    }

    pub fn set_len(&mut self, place: Place<'tcx>, len: Internval) {
        self.len.insert(place, len);
    }

    pub fn clear_len(&mut self, place: &Place<'tcx>) {
        self.len.remove(place);
    }

    pub fn all_fact_places(&self) -> impl Iterator<Item = Place<'tcx>> + '_ {
        self.internval.keys().chain(self.len.keys()).copied()
    }
}

pub fn join_state<'tcx>(
    a: &InternvalState<'tcx>,
    b: &InternvalState<'tcx>,
) -> InternvalState<'tcx> {
    let mut out = InternvalState::default();
    for k in a.internval.keys().chain(b.internval.keys()) {
        let ia = a.internval.get(k).copied().unwrap_or_else(Internval::empty);
        let ib = b.internval.get(k).copied().unwrap_or_else(Internval::empty);
        out.internval.insert(*k, join(&ia, &ib));
    }
    for k in a.len.keys().chain(b.len.keys()) {
        let ia = a.len.get(k).copied().unwrap_or_else(Internval::empty);
        let ib = b.len.get(k).copied().unwrap_or_else(Internval::empty);
        out.len.insert(*k, join(&ia, &ib));
    }
    out.eq = join_eq(&a.eq, &b.eq);
    out
}

pub fn widen_state<'tcx>(
    a: &InternvalState<'tcx>,
    b: &InternvalState<'tcx>,
) -> InternvalState<'tcx> {
    let mut out = InternvalState::default();
    for k in a.internval.keys().chain(b.internval.keys()) {
        let ia = a.internval.get(k).copied().unwrap_or_else(Internval::empty);
        let ib = b.internval.get(k).copied().unwrap_or_else(Internval::empty);
        let widened = widen(&ia, &ib);
        out.internval.insert(*k, widened);
    }
    for k in a.len.keys().chain(b.len.keys()) {
        let ia = a.len.get(k).copied().unwrap_or_else(Internval::empty);
        let ib = b.len.get(k).copied().unwrap_or_else(Internval::empty);
        out.len.insert(*k, widen(&ia, &ib));
    }
    out.eq = join_eq(&a.eq, &b.eq);
    out
}
