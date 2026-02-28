use crate::framework::printer::StateEntries;
use crate::framework::forward::DomainState;
use crate::internval::eq_domain::join_eq;
use rustc_middle::mir::Place;
use std::collections::HashMap;
use std::fmt;

use super::abstract_value::{Internval, join, widen};
use super::eq_domain::EqDomain;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InternvalState<'tcx> {
    pub internval: HashMap<Place<'tcx>, Internval>,
    pub eq: EqDomain<'tcx>,
}

impl<'tcx> InternvalState<'tcx> {
    fn default() -> Self {
        InternvalState {
            internval: HashMap::new(),
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

        InternvalState { internval, eq }
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
        previous.internval != next.internval
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
}

impl<'tcx> InternvalState<'tcx> {
    pub fn get_internval(&self, place: &Place<'tcx>) -> Internval {
        self.internval.get(place).copied().unwrap()
    }
    pub fn set_internval(&mut self, place: Place<'tcx>, internval: Internval) {
        self.internval.insert(place, internval);
    }
}

pub fn join_state<'tcx>(
    a: &InternvalState<'tcx>,
    b: &InternvalState<'tcx>,
) -> InternvalState<'tcx> {
    let mut out = InternvalState::default();
    for k in a.internval.keys().chain(b.internval.keys()) {
        let ia = a.internval.get(k).copied().unwrap();
        let ib = b.internval.get(k).copied().unwrap();
        out.internval.insert(*k, join(&ia, &ib));
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
        let ia = a.internval.get(k).copied().unwrap();
        let ib = b.internval.get(k).copied().unwrap();
        let widened = widen(&ia, &ib);
        out.internval.insert(*k, widened);
    }
    out.eq = join_eq(&a.eq, &b.eq);
    // println!("{:?}\n and {:?}\n widen to {:?}\n", &a, &b, &out);
    out
}
