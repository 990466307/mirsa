use std::collections::HashMap;
use rustc_middle::mir::Local;

use super::abstract_value::{join, Sign};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SignState {
    pub locals: HashMap<usize, Sign>,
}

impl SignState {
    pub fn get_local(&self, l: Local) -> Sign {
        self.locals.get(&l.index()).copied().unwrap_or(Sign::Top)
    }
    pub fn set_local(&mut self, l: Local, s: Sign) {
        self.locals.insert(l.index(), s);
    }
}

pub fn join_state(a: &SignState, b: &SignState) -> SignState {
    let mut out = SignState::default();
    for k in a.locals.keys().chain(b.locals.keys()) {
        let sa = a.locals.get(k).copied().unwrap_or(Sign::Top);
        let sb = b.locals.get(k).copied().unwrap_or(Sign::Top);
        out.locals.insert(*k, join(sa, sb));
    }
    out
}
