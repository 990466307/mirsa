use mirsa_framework::access_path::AccessPath;
use mirsa_framework::forward::DomainState;
use mirsa_framework::printer::StateEntries;
use mirsa_relations::symbolic::{SymbolicState, join_display_places};
use rustc_middle::mir::{LocalDecls, Place};
use rustc_middle::ty::{TyCtxt, TyKind};
use std::collections::{HashMap, HashSet};
use std::fmt;

use super::abstract_value::{Interval, join, widen};
use super::float_interval::{FloatInterval, join as join_float, widen as widen_float};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IntervalState<'tcx> {
    interval: HashMap<AccessPath, Interval>,
    float_interval: HashMap<AccessPath, FloatInterval>,
    len: HashMap<AccessPath, Interval>,
    tracked_len: HashSet<AccessPath>,
    display_places: HashMap<AccessPath, Place<'tcx>>,
    debug: bool,
}

impl<'tcx> IntervalState<'tcx> {
    fn default(debug: bool) -> Self {
        IntervalState {
            interval: HashMap::new(),
            float_interval: HashMap::new(),
            len: HashMap::new(),
            tracked_len: HashSet::new(),
            display_places: HashMap::new(),
            debug,
        }
    }

    pub fn new_bot_state(
        tcx: TyCtxt<'tcx>,
        local_decls: &LocalDecls<'tcx>,
        places: &[Place<'tcx>],
        len_places: &[Place<'tcx>],
        arg_count: usize,
        debug: bool,
    ) -> Self {
        let mut interval = HashMap::new();
        let mut float_interval = HashMap::new();
        let mut len = HashMap::new();
        let mut tracked_len = HashSet::new();
        let mut display_places = HashMap::new();

        for place in len_places {
            let Some(path) = Self::path_for_place(*place) else {
                continue;
            };
            let local_idx = place.local.index();
            let value = if local_idx >= 1 && local_idx <= arg_count {
                Interval::top()
            } else {
                Interval::empty()
            };
            tracked_len.insert(path.clone());
            len.insert(path.clone(), value);
            display_places.insert(path, *place);
        }

        for place in places {
            if !matches!(
                place.ty(local_decls, tcx).ty.kind(),
                TyKind::Int(_) | TyKind::Uint(_) | TyKind::Bool | TyKind::Char
            ) {
                continue;
            }
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

        for place in len_places {
            if !matches!(place.ty(local_decls, tcx).ty.kind(), TyKind::Float(_)) {
                continue;
            }
            let Some(path) = Self::path_for_place(*place) else {
                continue;
            };
            let local_idx = place.local.index();
            let value = if local_idx >= 1 && local_idx <= arg_count {
                FloatInterval::top()
            } else {
                FloatInterval::bottom()
            };
            float_interval.insert(path.clone(), value);
            display_places.insert(path, *place);
        }

        IntervalState {
            interval,
            float_interval,
            len,
            tracked_len,
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
        previous.interval != next.interval
            || previous.float_interval != next.float_interval
            || previous.len != next.len
    }
}

impl<'tcx> fmt::Display for IntervalState<'tcx> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut entries: Vec<(String, String)> = self
            .interval
            .iter()
            .map(|(path, interval)| (path.to_string(), interval.to_string()))
            .collect();
        entries.extend(
            self.float_interval
                .iter()
                .map(|(path, interval)| (path.to_string(), interval.to_string())),
        );
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
        let mut entries: Vec<_> = self
            .interval
            .iter()
            .filter_map(|(path, interval)| {
                self.display_places
                    .get(path)
                    .map(|place| (*place, interval.to_string()))
            })
            .collect();
        entries.extend(self.float_interval.iter().filter_map(|(path, interval)| {
            self.display_places
                .get(path)
                .map(|place| (*place, interval.to_string()))
        }));
        entries
    }

    fn should_print_entry(&self, place: Place<'tcx>) -> bool {
        let Some(path) = Self::path_for_place(place) else {
            return false;
        };
        self.interval
            .get(&path)
            .is_some_and(|interval| !interval.is_empty())
            || self
                .float_interval
                .get(&path)
                .is_some_and(|interval| !interval.is_bottom())
    }
}

impl<'tcx> IntervalState<'tcx> {
    pub fn debug(&self, args: fmt::Arguments<'_>) {
        if self.debug {
            eprintln!("[interval] {args}");
        }
    }

    pub fn path_for_place(place: Place<'tcx>) -> Option<AccessPath> {
        AccessPath::from_place(place)
    }

    pub fn tracked_interval_resolved(
        &self,
        symbolic: &SymbolicState<'tcx>,
        place: &Place<'tcx>,
    ) -> Option<Interval> {
        let path = Self::path_for_place(*place)?;
        self.lookup_interval(&symbolic.normalize_path(&path))
    }

    pub fn read_interval_resolved(
        &mut self,
        symbolic: &SymbolicState<'tcx>,
        place: Place<'tcx>,
    ) -> Interval {
        let Some(path) = Self::path_for_place(place) else {
            return Interval::top();
        };
        let path = symbolic.normalize_path(&path);
        if let Some(value) = self.lookup_interval(&path) {
            if self.interval.contains_key(&path) && self.interval.get(&path).copied() != Some(value)
            {
                self.interval.insert(path.clone(), value);
                self.display_places.entry(path).or_insert(place);
            }
            return value;
        }

        self.debug(format_args!("untracked interval read {path}; using top"));
        Interval::top()
    }

    pub fn set_interval_resolved(
        &mut self,
        symbolic: &SymbolicState<'tcx>,
        place: Place<'tcx>,
        interval: Interval,
    ) {
        let Some(path) = Self::path_for_place(place) else {
            return;
        };
        let path = symbolic.normalize_path(&path);
        if !self.interval.contains_key(&path) {
            self.debug(format_args!("untracked interval write {path}; ignored"));
            return;
        }
        self.interval.insert(path.clone(), interval);
        self.display_places.entry(path).or_insert(place);
    }

    pub fn join_interval_resolved(
        &mut self,
        symbolic: &SymbolicState<'tcx>,
        place: Place<'tcx>,
        interval: Interval,
    ) {
        let Some(path) = Self::path_for_place(place) else {
            return;
        };
        let path = symbolic.normalize_path(&path);
        let Some(old) = self.interval.get(&path).copied() else {
            self.debug(format_args!(
                "untracked interval join-write {path}; ignored"
            ));
            return;
        };
        let value = join(&old, &interval);
        self.interval.insert(path.clone(), value);
        self.display_places.entry(path).or_insert(place);
    }

    pub fn tracked_float_interval_resolved(
        &self,
        symbolic: &SymbolicState<'tcx>,
        place: &Place<'tcx>,
    ) -> Option<FloatInterval> {
        let path = Self::path_for_place(*place)?;
        self.lookup_float_interval(&symbolic.normalize_path(&path))
    }

    pub fn read_float_interval_resolved(
        &mut self,
        symbolic: &SymbolicState<'tcx>,
        place: Place<'tcx>,
    ) -> FloatInterval {
        let Some(path) = Self::path_for_place(place) else {
            return FloatInterval::top();
        };
        let path = symbolic.normalize_path(&path);
        if let Some(value) = self.lookup_float_interval(&path) {
            if self.float_interval.contains_key(&path)
                && self.float_interval.get(&path).copied() != Some(value)
            {
                self.float_interval.insert(path.clone(), value);
                self.display_places.entry(path).or_insert(place);
            }
            return value;
        }

        self.debug(format_args!(
            "untracked float interval read {path}; using top"
        ));
        FloatInterval::top()
    }

    pub fn set_float_interval_resolved(
        &mut self,
        symbolic: &SymbolicState<'tcx>,
        place: Place<'tcx>,
        interval: FloatInterval,
    ) {
        let Some(path) = Self::path_for_place(place) else {
            return;
        };
        let path = symbolic.normalize_path(&path);
        if !self.float_interval.contains_key(&path) {
            self.debug(format_args!(
                "untracked float interval write {path}; ignored"
            ));
            return;
        }
        self.float_interval.insert(path.clone(), interval);
        self.display_places.entry(path).or_insert(place);
    }

    pub fn join_float_interval_resolved(
        &mut self,
        symbolic: &SymbolicState<'tcx>,
        place: Place<'tcx>,
        interval: FloatInterval,
    ) {
        let Some(path) = Self::path_for_place(place) else {
            return;
        };
        let path = symbolic.normalize_path(&path);
        let Some(old) = self.float_interval.get(&path).copied() else {
            self.debug(format_args!(
                "untracked float interval join-write {path}; ignored"
            ));
            return;
        };
        let value = join_float(&old, &interval);
        self.float_interval.insert(path.clone(), value);
        self.display_places.entry(path).or_insert(place);
    }

    pub fn get_len(&self, place: &Place<'tcx>) -> Option<Interval> {
        let path = Self::path_for_place(*place)?;
        self.len
            .get(&path)
            .copied()
            .filter(|value| !value.is_empty() && *value != Interval::top())
    }

    pub fn read_len_resolved_or_top(
        &mut self,
        symbolic: &SymbolicState<'tcx>,
        place: Place<'tcx>,
    ) -> Interval {
        let Some(path) = Self::path_for_place(place) else {
            return Interval::top();
        };
        let path = symbolic.normalize_path(&path);
        if !self.tracked_len.contains(&path) {
            self.debug(format_args!("untracked len read {path}; using top"));
            return Interval::top();
        }
        self.len.get(&path).copied().unwrap_or_else(Interval::top)
    }

    pub fn set_len_resolved(
        &mut self,
        symbolic: &SymbolicState<'tcx>,
        place: Place<'tcx>,
        len: Interval,
    ) {
        let Some(path) = Self::path_for_place(place) else {
            return;
        };
        let path = symbolic.normalize_path(&path);
        if !self.tracked_len.contains(&path) {
            self.debug(format_args!("untracked len write {path}; ignored"));
            return;
        }
        self.len.insert(path.clone(), len);
        self.display_places.entry(path).or_insert(place);
    }

    pub fn clear_len_resolved(&mut self, symbolic: &SymbolicState<'tcx>, place: &Place<'tcx>) {
        let Some(path) = Self::path_for_place(*place) else {
            return;
        };
        let path = symbolic.normalize_path(&path);
        if !self.tracked_len.contains(&path) {
            return;
        }
        self.len.remove(&path);
    }

    pub fn all_fact_places(&self) -> Vec<Place<'tcx>> {
        self.interval
            .keys()
            .chain(self.float_interval.keys())
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

    pub fn float_interval_places(&self) -> Vec<Place<'tcx>> {
        self.float_interval
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

    fn lookup_interval(&self, path: &AccessPath) -> Option<Interval> {
        let exact = self.interval.get(path).copied();
        let mut value = exact;

        for (candidate, candidate_value) in &self.interval {
            if candidate == path {
                continue;
            }
            if path.matches_pattern(candidate) {
                value = Some(join(
                    &value.unwrap_or_else(Interval::empty),
                    candidate_value,
                ));
            }
        }

        match (exact, value) {
            (None, Some(value)) if value.is_empty() => None,
            (_, value) => value,
        }
    }

    fn lookup_float_interval(&self, path: &AccessPath) -> Option<FloatInterval> {
        let exact = self.float_interval.get(path).copied();
        let mut value = exact;

        for (candidate, candidate_value) in &self.float_interval {
            if candidate == path {
                continue;
            }
            if path.matches_pattern(candidate) {
                value = Some(join_float(
                    &value.unwrap_or_else(FloatInterval::bottom),
                    candidate_value,
                ));
            }
        }

        match (exact, value) {
            (None, Some(value)) if value.is_bottom() => None,
            (_, value) => value,
        }
    }
}

pub fn join_state<'tcx>(a: &IntervalState<'tcx>, b: &IntervalState<'tcx>) -> IntervalState<'tcx> {
    let mut out = IntervalState::default(a.debug || b.debug);
    for k in a.interval.keys().chain(b.interval.keys()) {
        let ia = a.interval.get(k).copied().unwrap_or_else(Interval::empty);
        let ib = b.interval.get(k).copied().unwrap_or_else(Interval::empty);
        out.interval.insert(k.clone(), join(&ia, &ib));
    }
    for k in a.float_interval.keys().chain(b.float_interval.keys()) {
        let ia = a
            .float_interval
            .get(k)
            .copied()
            .unwrap_or_else(FloatInterval::bottom);
        let ib = b
            .float_interval
            .get(k)
            .copied()
            .unwrap_or_else(FloatInterval::bottom);
        out.float_interval.insert(k.clone(), join_float(&ia, &ib));
    }
    out.tracked_len = a.tracked_len.union(&b.tracked_len).cloned().collect();
    for k in &out.tracked_len {
        let ia = a.len.get(k).copied().unwrap_or_else(Interval::top);
        let ib = b.len.get(k).copied().unwrap_or_else(Interval::top);
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
    for k in a.float_interval.keys().chain(b.float_interval.keys()) {
        let ia = a
            .float_interval
            .get(k)
            .copied()
            .unwrap_or_else(FloatInterval::bottom);
        let ib = b
            .float_interval
            .get(k)
            .copied()
            .unwrap_or_else(FloatInterval::bottom);
        out.float_interval.insert(k.clone(), widen_float(&ia, &ib));
    }
    out.tracked_len = a.tracked_len.union(&b.tracked_len).cloned().collect();
    for k in &out.tracked_len {
        let ia = a.len.get(k).copied().unwrap_or_else(Interval::top);
        let ib = b.len.get(k).copied().unwrap_or_else(Interval::top);
        out.len.insert(k.clone(), widen(&ia, &ib));
    }
    out.display_places = join_display_places(&a.display_places, &b.display_places);
    out
}
