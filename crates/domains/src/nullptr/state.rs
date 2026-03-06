use crate::framework::forward::DomainState;
use crate::framework::printer::StateEntries;
use rustc_middle::mir::Place;
use std::collections::HashMap;
use std::fmt;

use super::abstract_value::{NullPtr, join};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NullPtrState<'tcx> {
    pub pointers: HashMap<Place<'tcx>, NullPtr>,
    pub refs: HashMap<Place<'tcx>, NullPtr>,
}

impl<'tcx> NullPtrState<'tcx> {
    fn default() -> Self {
        NullPtrState {
            pointers: HashMap::new(),
            refs: HashMap::new(),
        }
    }

    pub fn new_bot_state(
        pointer_places: &[Place<'tcx>],
        ref_places: &[Place<'tcx>],
        arg_count: usize,
    ) -> Self {
        let mut pointers = HashMap::new();
        let mut refs = HashMap::new();
        for place in pointer_places {
            let local_idx = place.local.index();
            let value = if local_idx >= 1 && local_idx <= arg_count {
                NullPtr::MaybeNull
            } else {
                NullPtr::Bot
            };
            pointers.insert(*place, value);
        }

        for ref_item in ref_places {
            let _ = arg_count;
            refs.insert(*ref_item, NullPtr::Bot);
        }

        NullPtrState { pointers, refs }
    }

    pub fn get_nullptr(&self, place: &Place<'tcx>) -> NullPtr {
        self.pointers.get(place).copied().unwrap_or(NullPtr::Bot)
    }

    pub fn set_nullptr(&mut self, place: Place<'tcx>, value: NullPtr) {
        self.pointers.insert(place, value);
    }

    pub fn get_ref(&self, place: &Place<'tcx>) -> NullPtr {
        self.refs.get(place).copied().unwrap_or(NullPtr::Bot)
    }

    pub fn set_ref(&mut self, place: Place<'tcx>, value: NullPtr) {
        self.refs.insert(place, value);
    }
}

impl<'tcx> DomainState<'tcx> for NullPtrState<'tcx> {
    fn join(left: &Self, right: &Self) -> Self {
        join_state(left, right)
    }

    fn state_changed(previous: &Self, next: &Self) -> bool {
        previous.pointers != next.pointers || previous.refs != next.refs
    }
}

impl<'tcx> fmt::Display for NullPtrState<'tcx> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut entries: Vec<(String, String)> = self
            .pointers
            .iter()
            .map(|(place, ptr)| (format!("{place:?}"), ptr.to_string()))
            .collect();
        entries.extend(
            self.refs
                .iter()
                .map(|(place, ptr)| (format!("{place:?}"), format!("ref:{ptr}"))),
        );
        entries.sort_by(|a, b| a.0.cmp(&b.0));

        for (idx, (place, ptr)) in entries.iter().enumerate() {
            if idx > 0 {
                write!(f, ", ")?;
            }
            write!(f, "{place} => {ptr}")?;
        }
        Ok(())
    }
}

impl<'tcx> StateEntries<'tcx> for NullPtrState<'tcx> {
    fn entries(&self) -> Vec<(Place<'tcx>, String)> {
        let mut out: Vec<(Place<'tcx>, String)> = self
            .pointers
            .iter()
            .map(|(place, ptr)| (*place, ptr.to_string()))
            .collect();
        out.extend(
            self.refs
                .iter()
                .map(|(place, ptr)| (*place, format!("ref:{ptr}"))),
        );
        out
    }

    fn should_print_entry(&self, place: Place<'tcx>) -> bool {
        self.pointers
            .get(&place)
            .is_some_and(|v| *v != NullPtr::Bot)
            || self.refs.get(&place).is_some_and(|v| *v != NullPtr::Bot)
    }
}

pub fn join_state<'tcx>(a: &NullPtrState<'tcx>, b: &NullPtrState<'tcx>) -> NullPtrState<'tcx> {
    let mut out = NullPtrState::default();
    for k in a.pointers.keys().chain(b.pointers.keys()) {
        let va = a.pointers.get(k).copied().unwrap_or(NullPtr::Bot);
        let vb = b.pointers.get(k).copied().unwrap_or(NullPtr::Bot);
        out.pointers.insert(*k, join(va, vb));
    }
    for k in a.refs.keys().chain(b.refs.keys()) {
        let va = a.refs.get(k).copied().unwrap_or(NullPtr::Bot);
        let vb = b.refs.get(k).copied().unwrap_or(NullPtr::Bot);
        out.refs.insert(*k, join(va, vb));
    }
    out
}
