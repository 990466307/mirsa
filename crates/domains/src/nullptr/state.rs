use crate::framework::eq_domain::{EqDomain, join_eq};
use crate::framework::forward::DomainState;
use crate::framework::printer::StateEntries;
use rustc_middle::mir::Place;
use std::collections::{HashMap, HashSet};
use std::fmt;

use super::abstract_value::{NullPtr, join};
use super::access_path::{AccessPath, AccessPathElem};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NullPtrState<'tcx> {
    facts: HashMap<AccessPath, NullPtr>,
    display_places: HashMap<AccessPath, Place<'tcx>>,
    pub eq: EqDomain<'tcx, AccessPath>,
    debug: bool,
}

impl<'tcx> NullPtrState<'tcx> {
    fn default(debug: bool) -> Self {
        Self {
            facts: HashMap::new(),
            display_places: HashMap::new(),
            eq: EqDomain::new(),
            debug,
        }
    }

    pub fn new_bot_state(pointer_places: &[Place<'tcx>], arg_count: usize, debug: bool) -> Self {
        let mut state = Self::default(debug);
        for place in pointer_places {
            let Some(path) = Self::path_for_place(*place) else {
                continue;
            };
            let local_idx = place.local.index();
            let value = if local_idx >= 1 && local_idx <= arg_count {
                NullPtr::MaybeNull
            } else {
                NullPtr::Bot
            };
            state.facts.insert(path.clone(), value);
            state.display_places.insert(path, *place);
        }
        state
    }

    pub fn debug(&self, args: fmt::Arguments<'_>) {
        if self.debug {
            eprintln!("[nullptr] {args}");
        }
    }

    pub fn debug_map(&self, label: &str) {
        if !self.debug {
            return;
        }

        let mut entries: Vec<_> = self
            .facts
            .iter()
            .map(|(path, value)| (path.to_string(), value.to_string()))
            .collect();
        entries.sort_by(|a, b| a.0.cmp(&b.0));

        eprintln!("[nullptr] {label}:");
        if entries.is_empty() {
            eprintln!("[nullptr]   <empty>");
            return;
        }
        for (path, value) in entries {
            eprintln!("[nullptr]   {path} => {value}");
        }
    }

    pub fn access_path_for_place(&self, place: Place<'tcx>) -> Option<AccessPath> {
        Self::path_for_place(place)
    }

    pub fn path_for_place(place: Place<'tcx>) -> Option<AccessPath> {
        AccessPath::from_projection(AccessPath::from_local(place.local), place.projection)
    }

    pub fn contains_place(&self, place: Place<'tcx>) -> bool {
        self.access_path_for_place(place)
            .is_some_and(|path| self.facts.contains_key(&path))
    }

    pub fn get_path(&self, path: &AccessPath) -> NullPtr {
        self.facts.get(path).copied().unwrap_or(NullPtr::Bot)
    }

    pub fn value_or_maybe(&self, path: &AccessPath) -> NullPtr {
        if let Some(value) = self.facts.get(path).copied() {
            return value;
        }

        let mut found = false;
        let mut value = NullPtr::Bot;
        for (candidate, candidate_value) in &self.facts {
            if candidate.matches_pattern(path) {
                found = true;
                value = join(value, *candidate_value);
            }
        }
        if found { value } else { NullPtr::MaybeNull }
    }

    pub fn set_path(&mut self, path: AccessPath, value: NullPtr) {
        self.set_path_with_place(path, None, value);
    }

    pub fn set_place_path(&mut self, place: Place<'tcx>, value: NullPtr) {
        if let Some(path) = self.access_path_for_place(place) {
            self.set_path_with_place(path, Some(place), value);
        }
    }

    fn set_path_with_place(
        &mut self,
        path: AccessPath,
        display_place: Option<Place<'tcx>>,
        value: NullPtr,
    ) {
        let old = self.facts.insert(path.clone(), value);
        if old != Some(value) && value != NullPtr::Bot {
            self.debug(format_args!("fact {path} := {value}"));
        }
        if let Some(place) = display_place {
            self.display_places.insert(path, place);
        }
    }

    pub fn copy_place_from_path(
        &mut self,
        place: Place<'tcx>,
        src: &AccessPath,
        default: NullPtr,
        reason: &str,
    ) {
        let Some(dst) = self.access_path_for_place(place) else {
            return;
        };
        self.display_places.insert(dst.clone(), place);
        self.copy_subtree(&dst, src, default, reason);
    }

    pub fn copy_subtree(
        &mut self,
        dst: &AccessPath,
        src: &AccessPath,
        default: NullPtr,
        reason: &str,
    ) {
        self.copy_subtree_impl(dst, src, default, reason, true);
    }

    pub fn copy_child_subtree(
        &mut self,
        dst: &AccessPath,
        src: &AccessPath,
        default: NullPtr,
        reason: &str,
    ) {
        self.copy_subtree_impl(dst, src, default, reason, false);
    }

    fn copy_subtree_impl(
        &mut self,
        dst: &AccessPath,
        src: &AccessPath,
        default: NullPtr,
        reason: &str,
        include_root: bool,
    ) {
        self.debug(format_args!("{reason} {dst} <- {src}"));
        let mut suffixes: HashSet<Vec<AccessPathElem>> = HashSet::new();
        if include_root {
            suffixes.insert(Vec::new());
        }
        for path in self.facts.keys() {
            if let Some(suffix) = path.strip_pattern_prefix(dst) {
                if include_root || !suffix.is_empty() {
                    suffixes.insert(suffix);
                }
            }
            if let Some(suffix) = path.strip_pattern_prefix(src) {
                if include_root || !suffix.is_empty() {
                    suffixes.insert(suffix);
                }
            }
        }

        let mut updates = Vec::new();
        for suffix in suffixes {
            let dst_path = dst.join_suffix(&suffix);
            let src_path = src.join_suffix(&suffix);
            let value = match self.value_or_maybe(&src_path) {
                NullPtr::Bot => default,
                value => value,
            };
            updates.push((dst_path, value));
        }

        for (path, value) in updates {
            self.write_pattern(path, value);
        }
    }

    fn write_pattern(&mut self, pattern: AccessPath, value: NullPtr) {
        if !pattern.has_index() {
            self.set_path(pattern, value);
            return;
        }

        let targets: Vec<_> = self
            .facts
            .keys()
            .filter(|candidate| candidate.matches_pattern(&pattern))
            .cloned()
            .collect();
        if targets.is_empty() {
            self.set_path(pattern, value);
            return;
        }
        for target in targets {
            let current = self.get_path(&target);
            self.set_path(target, join(current, value));
        }
    }

    pub fn fact_paths(&self) -> impl Iterator<Item = AccessPath> + '_ {
        self.facts.keys().cloned()
    }

    fn is_bottom_like(&self) -> bool {
        self.facts.values().all(|value| *value == NullPtr::Bot)
    }
}

impl<'tcx> DomainState<'tcx> for NullPtrState<'tcx> {
    fn join(left: &Self, right: &Self) -> Self {
        join_state(left, right)
    }

    fn state_changed(previous: &Self, next: &Self) -> bool {
        previous.facts != next.facts || !previous.eq.equivalent_to(&next.eq)
    }
}

impl<'tcx> fmt::Display for NullPtrState<'tcx> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut entries: Vec<(String, String)> = self
            .facts
            .iter()
            .map(|(path, value)| (path.to_string(), value.to_string()))
            .collect();
        entries.sort_by(|a, b| a.0.cmp(&b.0));

        for (idx, (path, value)) in entries.iter().enumerate() {
            if idx > 0 {
                write!(f, ", ")?;
            }
            write!(f, "{path} => {value}")?;
        }
        Ok(())
    }
}

impl<'tcx> StateEntries<'tcx> for NullPtrState<'tcx> {
    fn entries(&self) -> Vec<(Place<'tcx>, String)> {
        self.display_places
            .iter()
            .filter_map(|(path, place)| {
                let value = self.get_path(path);
                if value == NullPtr::Bot {
                    None
                } else {
                    Some((*place, value.to_string()))
                }
            })
            .collect()
    }

    fn should_print_entry(&self, place: Place<'tcx>) -> bool {
        let Some(path) = self.access_path_for_place(place) else {
            return false;
        };
        self.get_path(&path) != NullPtr::Bot
    }
}

pub fn join_state<'tcx>(a: &NullPtrState<'tcx>, b: &NullPtrState<'tcx>) -> NullPtrState<'tcx> {
    if a.is_bottom_like() {
        return b.clone();
    }
    if b.is_bottom_like() {
        return a.clone();
    }

    let mut out = NullPtrState::default(a.debug || b.debug);
    for key in a.facts.keys().chain(b.facts.keys()) {
        let left_value = a.facts.get(key).copied().unwrap_or(NullPtr::Bot);
        let right_value = b.facts.get(key).copied().unwrap_or(NullPtr::Bot);
        out.facts.insert(key.clone(), join(left_value, right_value));
    }
    for key in a.display_places.keys().chain(b.display_places.keys()) {
        if let Some(place) = a
            .display_places
            .get(key)
            .or_else(|| b.display_places.get(key))
        {
            out.display_places.insert(key.clone(), *place);
        }
    }
    out.eq = join_eq(&a.eq, &b.eq);
    out
}
