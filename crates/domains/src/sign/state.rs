use crate::framework::forward::DomainState;
use crate::framework::printer::StateEntries;
use rustc_middle::mir::Place;
use std::collections::HashMap;
use std::fmt;

use super::abstract_value::{Sign, join};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SignState<'tcx> {
    pub locals: HashMap<Place<'tcx>, Sign>,
}

impl<'tcx> SignState<'tcx> {
    pub fn default() -> Self {
        SignState {
            locals: HashMap::new(),
        }
    }
    pub fn new_bot_state(places: &[Place<'tcx>]) -> Self {
        let mut locals = HashMap::new();

        for place in places {
            locals.insert(*place, Sign::Bot);
        }

        SignState { locals }
    }
}

impl<'tcx> DomainState<'tcx> for SignState<'tcx> {
    fn join(left: &Self, right: &Self) -> Self {
        join_state(left, right)
    }
}

impl<'tcx> fmt::Display for SignState<'tcx> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut entries: Vec<(String, String)> = self
            .locals
            .iter()
            .map(|(place, sign)| (format!("{place:?}"), format!("{sign:?}")))
            .collect();
        entries.sort_by(|a, b| a.0.cmp(&b.0));

        for (idx, (place, sign)) in entries.iter().enumerate() {
            if idx > 0 {
                write!(f, ", ")?;
            }
            write!(f, "{place} => {sign}")?;
        }
        Ok(())
    }
}

impl<'tcx> StateEntries<'tcx> for SignState<'tcx> {
    fn entries(&self) -> Vec<(Place<'tcx>, String)> {
        self.locals
            .iter()
            .map(|(place, sign)| (*place, format!("{sign:?}")))
            .collect()
    }
}

impl<'tcx> SignState<'tcx> {
    pub fn get_sign(&self, place: &Place<'tcx>) -> Sign {
        self.locals
            .get(place)
            .copied()
            .expect("place should be initialized in SignState")
    }
    pub fn set_sign(&mut self, place: Place<'tcx>, s: Sign) {
        self.locals.insert(place, s);
    }
}

pub fn join_state<'tcx>(a: &SignState<'tcx>, b: &SignState<'tcx>) -> SignState<'tcx> {
    let mut out = SignState::default();
    for k in a.locals.keys().chain(b.locals.keys()) {
        let sa = a.locals.get(k).copied().unwrap();
        let sb = b.locals.get(k).copied().unwrap();
        out.locals.insert(*k, join(sa, sb));
    }
    out
}
