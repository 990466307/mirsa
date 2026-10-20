use mirsa_framework::access_path::AccessPath;
use mirsa_framework::forward::DomainState;
use mirsa_framework::mapping::update_value_children;
use mirsa_framework::printer::StateEntries;
use mirsa_framework::revision::{Revision, Versioned};
use mirsa_relations::symbolic::{SymbolicState, join_display_places};
use rustc_middle::mir::{LocalDecls, Place};
use rustc_middle::ty::{ScalarInt, TyCtxt, TyKind};
use std::collections::{HashMap, HashSet};
use std::fmt;

use super::abstract_value::{Interval, join, widen};
use super::float_interval::{FloatInterval, join as join_float, widen as widen_float};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IntervalState<'tcx> {
    interval: Versioned<HashMap<AccessPath, Interval>>,
    integer_bits: Versioned<HashMap<AccessPath, ScalarInt>>,
    enum_variants: Versioned<HashMap<AccessPath, usize>>,
    float_interval: Versioned<HashMap<AccessPath, FloatInterval>>,
    len: Versioned<HashMap<AccessPath, Interval>>,
    tracked_len: Versioned<HashSet<AccessPath>>,
    display_places: Versioned<HashMap<AccessPath, Place<'tcx>>>,
    debug: bool,
}

pub enum IntervalFact<'a> {
    Range(&'a AccessPath, &'a Interval),
    Bits(&'a AccessPath, &'a ScalarInt),
    Variant(&'a AccessPath, usize),
    Float(&'a AccessPath, &'a FloatInterval),
    Length(&'a AccessPath, &'a Interval),
}

impl<'tcx> IntervalState<'tcx> {
    pub fn export_facts(&self, roots: &[AccessPath]) -> Vec<IntervalFact<'_>> {
        use mirsa_framework::facts::selected;
        let mut out = Vec::new();
        for (p, v) in &self.interval {
            if selected(p, roots) {
                out.push(IntervalFact::Range(p, v));
            }
        }
        for (p, v) in &self.integer_bits {
            if selected(p, roots) {
                out.push(IntervalFact::Bits(p, v));
            }
        }
        for (p, v) in &self.enum_variants {
            if selected(p, roots) {
                out.push(IntervalFact::Variant(p, *v));
            }
        }
        for (p, v) in &self.float_interval {
            if selected(p, roots) {
                out.push(IntervalFact::Float(p, v));
            }
        }
        for (p, v) in &self.len {
            if selected(p, roots) {
                out.push(IntervalFact::Length(p, v));
            }
        }
        out
    }

    pub fn project(&self, map: &impl mirsa_framework::mapping::PathMapping<'tcx>) -> Self {
        Self {
            interval: map.values(&self.interval).into(),
            integer_bits: map.values(&self.integer_bits).into(),
            enum_variants: map.values(&self.enum_variants).into(),
            float_interval: map.values(&self.float_interval).into(),
            len: map.values(&self.len).into(),
            tracked_len: map.paths(&self.tracked_len).into(),
            display_places: map.places(&self.display_places).into(),
            debug: self.debug,
        }
    }

    pub fn extend(&mut self, other: Self) {
        self.interval.extend(other.interval);
        self.integer_bits.extend(other.integer_bits);
        self.enum_variants.extend(other.enum_variants);
        self.float_interval.extend(other.float_interval);
        self.len.extend(other.len);
        self.tracked_len.extend(other.tracked_len);
        self.display_places.extend(other.display_places);
    }

    pub fn enum_variant_resolved(
        &self,
        symbolic: &SymbolicState<'tcx>,
        place: Place<'tcx>,
    ) -> Option<usize> {
        self.enum_variants
            .get(&symbolic.normalize_place(place)?)
            .copied()
    }

    pub fn set_enum_variant_resolved(
        &mut self,
        symbolic: &SymbolicState<'tcx>,
        place: Place<'tcx>,
        variant: usize,
    ) {
        if let Some(path) = symbolic.normalize_place(place) {
            self.enum_variants.insert(path, variant);
        }
    }

    pub fn integer_bits_resolved(
        &self,
        symbolic: &SymbolicState<'tcx>,
        place: Place<'tcx>,
    ) -> Option<ScalarInt> {
        self.integer_bits
            .get(&symbolic.normalize_place(place)?)
            .copied()
    }

    pub fn set_integer_bits_resolved(
        &mut self,
        symbolic: &SymbolicState<'tcx>,
        place: Place<'tcx>,
        value: ScalarInt,
    ) {
        if let Some(path) = symbolic.normalize_place(place) {
            self.integer_bits.insert(path, value);
        }
    }

    pub fn invalidate_value_facts(&mut self, symbolic: &SymbolicState<'tcx>, paths: &[AccessPath]) {
        self.integer_bits
            .retain(|path, _| !symbolic.write_affects(path, paths));
        self.enum_variants
            .retain(|path, _| !symbolic.write_affects(path, paths));
    }

    pub fn invalidate_paths(&mut self, symbolic: &SymbolicState<'tcx>, paths: &[AccessPath]) {
        self.invalidate_value_facts(symbolic, paths);
        for (path, value) in &mut self.interval {
            if symbolic.write_affects(path, paths) {
                *value = Interval::top();
            }
        }
        for (path, value) in &mut self.float_interval {
            if symbolic.write_affects(path, paths) {
                *value = FloatInterval::top();
            }
        }
        for (path, value) in &mut self.len {
            if symbolic.write_affects(path, paths) {
                *value = Interval::top();
            }
        }
    }

    pub(super) fn invalidate_children(&mut self, destination: &AccessPath) {
        update_value_children(&mut self.interval, destination, |_| Interval::top());
        update_value_children(&mut self.float_interval, destination, |_| {
            FloatInterval::top()
        });
        update_value_children(&mut self.len, destination, |_| Interval::top());
    }

    pub(super) fn copy_children_from(
        &mut self,
        source: &Self,
        destination: &AccessPath,
        origin: &AccessPath,
    ) {
        update_value_children(&mut self.interval, destination, |suffix| {
            source
                .lookup_interval(&origin.join_suffix(suffix))
                .unwrap_or_else(Interval::top)
        });
        update_value_children(&mut self.float_interval, destination, |suffix| {
            source
                .lookup_float_interval(&origin.join_suffix(suffix))
                .unwrap_or_else(FloatInterval::top)
        });
        update_value_children(&mut self.len, destination, |suffix| {
            source
                .len
                .get(&origin.join_suffix(suffix))
                .copied()
                .unwrap_or_else(Interval::top)
        });
        self.integer_bits
            .extend(source.integer_bits.iter().filter_map(|(path, value)| {
                Some((
                    destination.join_suffix(path.value_suffix(origin).filter(|s| !s.is_empty())?),
                    *value,
                ))
            }));
        self.enum_variants
            .extend(source.enum_variants.iter().filter_map(|(path, value)| {
                Some((
                    destination.join_suffix(path.value_suffix(origin).filter(|s| !s.is_empty())?),
                    *value,
                ))
            }));
    }

    pub(super) fn clear_index_summaries(
        &mut self,
        symbolic: &SymbolicState<'tcx>,
        place: Place<'tcx>,
    ) {
        let Some(path) = symbolic.normalize_place(place) else {
            return;
        };
        let is_summary = |candidate: &AccessPath| {
            candidate.strip_pattern_prefix(&path).is_some_and(|suffix| {
                suffix.contains(&mirsa_framework::access_path::AccessPathElem::Index)
            })
        };
        for (candidate, value) in &mut self.interval {
            if is_summary(candidate) {
                *value = Interval::empty();
            }
        }
        for (candidate, value) in &mut self.float_interval {
            if is_summary(candidate) {
                *value = FloatInterval::bottom();
            }
        }
    }

    fn default(debug: bool) -> Self {
        IntervalState {
            interval: HashMap::new().into(),
            integer_bits: HashMap::new().into(),
            enum_variants: HashMap::new().into(),
            float_interval: HashMap::new().into(),
            len: HashMap::new().into(),
            tracked_len: HashSet::new().into(),
            display_places: HashMap::new().into(),
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
            interval: interval.into(),
            integer_bits: HashMap::new().into(),
            enum_variants: HashMap::new().into(),
            float_interval: float_interval.into(),
            len: len.into(),
            tracked_len: tracked_len.into(),
            display_places: display_places.into(),
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
                self.display_places.insert_if_absent(path, place);
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
        self.set_interval_path(path, place, interval);
    }

    pub(super) fn set_variant_path(&mut self, path: AccessPath, variant: usize) {
        self.enum_variants.insert(path, variant);
    }

    pub(super) fn set_interval_path(
        &mut self,
        path: AccessPath,
        place: Place<'tcx>,
        interval: Interval,
    ) {
        if !self.interval.contains_key(&path) {
            self.debug(format_args!("untracked interval write {path}; ignored"));
            return;
        }
        self.interval.insert(path.clone(), interval);
        self.display_places.insert_if_absent(path, place);
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
        self.display_places.insert_if_absent(path, place);
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
                (*self.float_interval).insert(path.clone(), value);
                self.display_places.insert_if_absent(path, place);
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
        // Numeric equality identifies signed zeros; mutable access conservatively
        // invalidates reduction caches even for such representation changes.
        (*self.float_interval).insert(path.clone(), interval);
        self.display_places.insert_if_absent(path, place);
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
        (*self.float_interval).insert(path.clone(), value);
        self.display_places.insert_if_absent(path, place);
    }

    pub fn length_places(&self) -> Vec<Place<'tcx>> {
        self.tracked_len
            .iter()
            .filter_map(|path| self.display_places.get(path).copied())
            .collect()
    }

    pub fn get_len(&self, place: &Place<'tcx>) -> Option<Interval> {
        let path = Self::path_for_place(*place)?;
        self.len
            .get(&path)
            .copied()
            .filter(|value| !value.is_empty() && *value != Interval::top())
    }

    pub fn tracked_len_resolved(
        &self,
        symbolic: &SymbolicState<'tcx>,
        place: Place<'tcx>,
    ) -> Option<Interval> {
        self.len.get(&symbolic.normalize_place(place)?).copied()
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
        self.display_places.insert_if_absent(path, place);
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

        let Some(patterns) = path.matching_patterns() else {
            return Some(Interval::top());
        };
        for candidate in patterns.into_iter().skip(1) {
            if let Some(candidate_value) = self.interval.get(&candidate) {
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

        let Some(patterns) = path.matching_patterns() else {
            return Some(FloatInterval::top());
        };
        for candidate in patterns.into_iter().skip(1) {
            if let Some(candidate_value) = self.float_interval.get(&candidate) {
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
    out.integer_bits = a
        .integer_bits
        .iter()
        .filter(|(path, value)| b.integer_bits.get(*path) == Some(*value))
        .map(|(path, value)| (path.clone(), *value))
        .collect();
    out.enum_variants = a
        .enum_variants
        .iter()
        .filter(|(path, value)| b.enum_variants.get(*path) == Some(*value))
        .map(|(path, value)| (path.clone(), *value))
        .collect();
    out.display_places = join_display_places(&a.display_places, &b.display_places).into();
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
    out.integer_bits = a
        .integer_bits
        .iter()
        .filter(|(path, value)| b.integer_bits.get(*path) == Some(*value))
        .map(|(path, value)| (path.clone(), *value))
        .collect();
    out.enum_variants = a
        .enum_variants
        .iter()
        .filter(|(path, value)| b.enum_variants.get(*path) == Some(*value))
        .map(|(path, value)| (path.clone(), *value))
        .collect();
    out.display_places = join_display_places(&a.display_places, &b.display_places).into();
    out
}

impl<'tcx> IntervalState<'tcx> {
    pub fn revision(&self) -> [Revision; 7] {
        [
            self.interval.revision(),
            self.integer_bits.revision(),
            self.enum_variants.revision(),
            self.float_interval.revision(),
            self.len.revision(),
            self.tracked_len.revision(),
            self.display_places.revision(),
        ]
    }
}
