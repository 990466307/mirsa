use super::abstract_value::{
    AbstractBool, AllocationFact, AllocationId, AllocationMultiplicity, AllocationOrigin,
    AllocationStatus, LayoutValue, PointerValue,
};
use crate::interval::abstract_value::{Interval, add};
use mirsa_framework::access_path::AccessPath;
use mirsa_framework::forward::DomainState;
use mirsa_framework::printer::StateEntries;
use mirsa_relations::symbolic::{SymbolicState, join_display_places};
use rustc_middle::mir::{Body, LocalDecls, Place};
use rustc_middle::ty::{Ty, TyCtxt, TyKind, TypingEnv};
use std::collections::{HashMap, HashSet};
use std::fmt;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AllocationState<'tcx> {
    pointers: HashMap<AccessPath, PointerValue>,
    layouts: HashMap<AccessPath, LayoutValue>,
    objects: HashMap<AllocationId, AllocationFact>,
    tracked_pointers: HashSet<AccessPath>,
    tracked_layouts: HashSet<AccessPath>,
    display_places: HashMap<AccessPath, Place<'tcx>>,
    bottom: bool,
    debug: bool,
}

impl<'tcx> AllocationState<'tcx> {
    fn empty(bottom: bool, debug: bool) -> Self {
        Self {
            pointers: HashMap::new(),
            layouts: HashMap::new(),
            objects: HashMap::new(),
            tracked_pointers: HashSet::new(),
            tracked_layouts: HashSet::new(),
            display_places: HashMap::new(),
            bottom,
            debug,
        }
    }

    pub fn bottom(debug: bool) -> Self {
        Self::empty(true, debug)
    }

    pub fn new_bottom(
        tcx: TyCtxt<'tcx>,
        local_decls: &LocalDecls<'tcx>,
        places: &[Place<'tcx>],
        debug: bool,
    ) -> Self {
        let mut state = Self::bottom(debug);
        state.collect_tracked_places(tcx, local_decls, places);
        state
    }

    pub fn new_entry(
        tcx: TyCtxt<'tcx>,
        body: &Body<'tcx>,
        places: &[Place<'tcx>],
        debug: bool,
    ) -> Self {
        let mut state = Self::empty(false, debug);
        state.collect_tracked_places(tcx, &body.local_decls, places);

        for (local, decl) in body.local_decls.iter_enumerated() {
            let extent = type_size(tcx, decl.ty).unwrap_or_else(Interval::top);
            state.objects.insert(
                AllocationId::Stack(local),
                AllocationFact::live(AllocationOrigin::Stack, extent),
            );
        }

        for place in places {
            if !place.projection.is_empty() {
                continue;
            }
            let local_idx = place.local.index();
            let is_arg = local_idx >= 1 && local_idx <= body.arg_count;
            let Some(path) = AccessPath::from_place(*place) else {
                continue;
            };
            let ty = place.ty(&body.local_decls, tcx).ty;
            if state.tracked_pointers.contains(&path) {
                let value = if is_arg {
                    match ty.kind() {
                        TyKind::Ref(_, pointee, _) => {
                            let id = AllocationId::External(place.local);
                            let minimum = type_size(tcx, *pointee)
                                .map(|size| Interval::new(size.low, i128::MAX))
                                .unwrap_or_else(Interval::top);
                            state.objects.insert(
                                id.clone(),
                                AllocationFact::live(AllocationOrigin::External, minimum),
                            );
                            PointerValue::target(id, Interval::new(0, 0))
                        }
                        TyKind::RawPtr(_, _) => PointerValue::top(),
                        TyKind::Adt(_, _) if is_non_null_ty(tcx, ty) => {
                            let id = AllocationId::External(place.local);
                            state.objects.insert(
                                id.clone(),
                                AllocationFact {
                                    origin: AllocationOrigin::External,
                                    status: AllocationStatus::top(),
                                    extent: Interval::top(),
                                    multiplicity: AllocationMultiplicity::one(),
                                },
                            );
                            PointerValue::join(
                                &PointerValue::target(id, Interval::top()),
                                &PointerValue::invalid_non_null(),
                            )
                        }
                        _ => PointerValue::bottom(),
                    }
                } else {
                    PointerValue::bottom()
                };
                state.pointers.insert(path.clone(), value);
            }
            if state.tracked_layouts.contains(&path) {
                state.layouts.insert(
                    path,
                    if is_arg {
                        LayoutValue::top()
                    } else {
                        LayoutValue::bottom()
                    },
                );
            }
        }
        state
    }

    fn collect_tracked_places(
        &mut self,
        tcx: TyCtxt<'tcx>,
        local_decls: &LocalDecls<'tcx>,
        places: &[Place<'tcx>],
    ) {
        for place in places {
            let Some(path) = AccessPath::from_place(*place) else {
                continue;
            };
            let ty = place.ty(local_decls, tcx).ty;
            if is_allocation_pointer(tcx, ty) {
                self.tracked_pointers.insert(path.clone());
                self.pointers
                    .entry(path.clone())
                    .or_insert_with(PointerValue::bottom);
            }
            if is_layout_ty(tcx, ty) {
                self.tracked_layouts.insert(path.clone());
                self.layouts
                    .entry(path.clone())
                    .or_insert_with(LayoutValue::bottom);
            }
            self.display_places.insert(path, *place);
        }
    }

    pub fn is_bottom(&self) -> bool {
        self.bottom
    }

    pub fn debug(&self, args: fmt::Arguments<'_>) {
        if self.debug {
            eprintln!("[allocation] {args}");
        }
    }

    pub fn pointer_path_resolved(
        &self,
        symbolic: &SymbolicState<'tcx>,
        place: Place<'tcx>,
    ) -> Option<AccessPath> {
        AccessPath::from_place(place).map(|path| symbolic.normalize_path(&path))
    }

    pub fn pointer_value(&self, path: &AccessPath) -> PointerValue {
        if self.bottom {
            return PointerValue::bottom();
        }
        self.lookup_pointer(path).unwrap_or_else(PointerValue::top)
    }

    pub fn pointer_value_resolved(
        &self,
        symbolic: &SymbolicState<'tcx>,
        place: Place<'tcx>,
    ) -> PointerValue {
        let Some(path) = self.pointer_path_resolved(symbolic, place) else {
            return PointerValue::top();
        };
        self.pointer_value(&path)
    }

    pub fn set_pointer(&mut self, path: AccessPath, value: PointerValue) {
        if !self.tracked_pointers.contains(&path) {
            self.debug(format_args!("untracked pointer write {path}; ignored"));
            return;
        }
        self.bottom = false;
        self.debug(format_args!("pointer {path} := {value}"));
        self.pointers.insert(path, value);
    }

    pub fn set_pointer_resolved(
        &mut self,
        symbolic: &SymbolicState<'tcx>,
        place: Place<'tcx>,
        value: PointerValue,
    ) {
        let Some(path) = self.pointer_path_resolved(symbolic, place) else {
            return;
        };
        self.display_places.entry(path.clone()).or_insert(place);
        self.set_pointer(path, value);
    }

    pub fn join_pointer_resolved(
        &mut self,
        symbolic: &SymbolicState<'tcx>,
        place: Place<'tcx>,
        value: PointerValue,
    ) {
        let Some(path) = self.pointer_path_resolved(symbolic, place) else {
            return;
        };
        if !self.tracked_pointers.contains(&path) {
            self.debug(format_args!("untracked pointer weak write {path}; ignored"));
            return;
        }
        let current = self
            .lookup_pointer(&path)
            .unwrap_or_else(PointerValue::bottom);
        let joined = PointerValue::join(&current, &value);
        self.display_places.entry(path.clone()).or_insert(place);
        self.set_pointer(path, joined);
    }

    pub fn kill_pointer_tree(&mut self, path: &AccessPath) {
        let targets: Vec<_> = self
            .tracked_pointers
            .iter()
            .filter(|candidate| candidate.strip_pattern_prefix(path).is_some())
            .cloned()
            .collect();
        for target in targets {
            self.pointers.insert(target, PointerValue::bottom());
        }
    }

    pub fn copy_pointer_tree(&mut self, dst: &AccessPath, src: &AccessPath, default: PointerValue) {
        let mut suffixes = HashSet::new();
        suffixes.insert(Vec::new());
        for path in &self.tracked_pointers {
            if let Some(suffix) = path.strip_pattern_prefix(dst) {
                suffixes.insert(suffix);
            }
            if let Some(suffix) = path.strip_pattern_prefix(src) {
                suffixes.insert(suffix);
            }
        }
        for suffix in suffixes {
            let dst_path = dst.join_suffix(&suffix);
            if !self.tracked_pointers.contains(&dst_path) {
                continue;
            }
            let src_path = src.join_suffix(&suffix);
            let value = self
                .lookup_pointer(&src_path)
                .unwrap_or_else(|| default.clone());
            self.set_pointer(dst_path, value);
        }
    }

    pub fn layout_value(&self, path: &AccessPath) -> LayoutValue {
        if self.bottom {
            return LayoutValue::bottom();
        }
        self.layouts
            .get(path)
            .copied()
            .unwrap_or_else(LayoutValue::top)
    }

    pub fn layout_value_resolved(
        &self,
        symbolic: &SymbolicState<'tcx>,
        place: Place<'tcx>,
    ) -> LayoutValue {
        let Some(path) = self.pointer_path_resolved(symbolic, place) else {
            return LayoutValue::top();
        };
        self.layout_value(&path)
    }

    pub fn set_layout_resolved(
        &mut self,
        symbolic: &SymbolicState<'tcx>,
        place: Place<'tcx>,
        value: LayoutValue,
    ) {
        let Some(path) = self.pointer_path_resolved(symbolic, place) else {
            return;
        };
        if !self.tracked_layouts.contains(&path) {
            return;
        }
        self.bottom = false;
        self.debug(format_args!("layout {path} := {value}"));
        self.layouts.insert(path.clone(), value);
        self.display_places.entry(path).or_insert(place);
    }

    pub fn object(&self, id: &AllocationId) -> Option<&AllocationFact> {
        self.objects.get(id)
    }

    pub fn objects(&self) -> impl Iterator<Item = (&AllocationId, &AllocationFact)> {
        self.objects.iter()
    }

    pub fn set_object(&mut self, id: AllocationId, fact: AllocationFact) {
        self.bottom = false;
        self.debug(format_args!("object {id} := {fact}"));
        self.objects.insert(id, fact);
    }

    pub fn allocate_fallibly(&mut self, id: AllocationId, extent: Interval) -> PointerValue {
        let old = self
            .objects
            .get(&id)
            .cloned()
            .unwrap_or_else(|| AllocationFact::absent(id.origin()));
        let multiplicity = old.multiplicity.after_allocation();
        let repeated = !multiplicity.is_exactly_one();
        let status = if repeated {
            old.status.join(AllocationStatus::live())
        } else {
            AllocationStatus::absent_or_live()
        };
        let known_extent = if old.extent.is_empty() {
            extent
        } else {
            crate::interval::abstract_value::join(&old.extent, &extent)
        };
        self.set_object(
            id.clone(),
            AllocationFact {
                origin: id.origin(),
                status,
                extent: known_extent,
                multiplicity,
            },
        );
        PointerValue::fallible_target(id, Interval::new(0, 0))
    }

    pub fn deallocate(&mut self, pointer: &PointerValue, conditional: bool) {
        if pointer.is_bottom() {
            return;
        }
        let exact = pointer.exact_target().map(|(id, _)| id.clone());
        let ids: Vec<_> = pointer.targets().map(|(id, _)| id.clone()).collect();
        for id in ids {
            let Some(old) = self.objects.get(&id).cloned() else {
                continue;
            };
            let strong =
                !conditional && exact.as_ref() == Some(&id) && old.multiplicity.is_exactly_one();
            let mut next = old;
            next.status = if strong {
                AllocationStatus::dead()
            } else {
                next.status.join(AllocationStatus::dead())
            };
            self.set_object(id, next);
        }

        if pointer.has_unknown_target() {
            let ids: Vec<_> = self
                .objects
                .keys()
                .filter(|id| {
                    matches!(
                        id.origin(),
                        AllocationOrigin::Heap | AllocationOrigin::External
                    )
                })
                .cloned()
                .collect();
            for id in ids {
                if let Some(mut fact) = self.objects.get(&id).cloned() {
                    fact.status = fact.status.join(AllocationStatus::dead());
                    self.set_object(id, fact);
                }
            }
        }
    }

    pub fn set_stack_live(&mut self, local: rustc_middle::mir::Local, live: bool) {
        let id = AllocationId::Stack(local);
        let Some(mut fact) = self.objects.get(&id).cloned() else {
            return;
        };
        fact.status = if live {
            AllocationStatus::live()
        } else {
            AllocationStatus::dead()
        };
        self.set_object(id, fact);
    }

    pub fn pointer_places(&self) -> Vec<Place<'tcx>> {
        self.tracked_pointers
            .iter()
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

    pub fn same_allocation(&self, left: &PointerValue, right: &PointerValue) -> AbstractBool {
        if left.is_bottom() || right.is_bottom() {
            return AbstractBool::Bottom;
        }
        if left.has_unknown_target() || right.has_unknown_target() {
            return AbstractBool::Top;
        }
        let mut result = AbstractBool::Bottom;
        if left.may_be_null()
            || right.may_be_null()
            || left.may_be_invalid_non_null()
            || right.may_be_invalid_non_null()
        {
            result = result.join(AbstractBool::False);
        }
        for (left_id, _) in left.targets() {
            for (right_id, _) in right.targets() {
                let relation = if left_id == right_id {
                    if self
                        .objects
                        .get(left_id)
                        .is_some_and(|fact| fact.multiplicity.is_exactly_one())
                    {
                        AbstractBool::True
                    } else {
                        AbstractBool::Top
                    }
                } else if left_id.may_alias(right_id) {
                    AbstractBool::Top
                } else {
                    AbstractBool::False
                };
                result = result.join(relation);
            }
        }
        result
    }

    pub fn is_live(&self, pointer: &PointerValue) -> AbstractBool {
        self.evaluate_pointer(pointer, |fact, _| {
            if fact.status.is_definitely_live() {
                AbstractBool::True
            } else if !fact.status.may_be_live() {
                AbstractBool::False
            } else {
                AbstractBool::Top
            }
        })
    }

    pub fn is_base(&self, pointer: &PointerValue) -> AbstractBool {
        self.evaluate_pointer(pointer, |_, offset| interval_is_zero(*offset))
    }

    pub fn range_in_allocation(&self, pointer: &PointerValue, len: Interval) -> AbstractBool {
        self.evaluate_pointer(pointer, |fact, offset| {
            let live = if fact.status.is_definitely_live() {
                AbstractBool::True
            } else if !fact.status.may_be_live() {
                AbstractBool::False
            } else {
                AbstractBool::Top
            };
            let bounds = range_within(*offset, len, fact.extent);
            match (live, bounds) {
                (AbstractBool::False, _) | (_, AbstractBool::False) => AbstractBool::False,
                (AbstractBool::True, AbstractBool::True) => AbstractBool::True,
                (AbstractBool::Bottom, value) | (value, AbstractBool::Bottom) => value,
                _ => AbstractBool::Top,
            }
        })
    }

    fn evaluate_pointer(
        &self,
        pointer: &PointerValue,
        mut evaluate: impl FnMut(&AllocationFact, &Interval) -> AbstractBool,
    ) -> AbstractBool {
        if pointer.is_bottom() {
            return AbstractBool::Bottom;
        }
        if pointer.has_unknown_target() {
            return AbstractBool::Top;
        }
        let mut result = AbstractBool::Bottom;
        if pointer.may_be_null() || pointer.may_be_invalid_non_null() {
            result = result.join(AbstractBool::False);
        }
        for (id, offset) in pointer.targets() {
            let value = self
                .objects
                .get(id)
                .map(|fact| evaluate(fact, offset))
                .unwrap_or(AbstractBool::Top);
            result = result.join(value);
        }
        result
    }

    pub fn constrain_equal_paths(
        &mut self,
        symbolic: &SymbolicState<'tcx>,
        left: Place<'tcx>,
        right: Place<'tcx>,
    ) -> bool {
        let (Some(left_path), Some(right_path)) = (
            self.pointer_path_resolved(symbolic, left),
            self.pointer_path_resolved(symbolic, right),
        ) else {
            return true;
        };
        let left_value = self.pointer_value(&left_path);
        let right_value = self.pointer_value(&right_path);
        let cross_summary_alias = left_value.targets().any(|(left_id, _)| {
            right_value
                .targets()
                .any(|(right_id, _)| left_id != right_id && left_id.may_alias(right_id))
        });
        if cross_summary_alias {
            return true;
        }
        let common = PointerValue::intersect_equal(&left_value, &right_value);
        if common.is_bottom() && !left_value.is_bottom() && !right_value.is_bottom() {
            return false;
        }
        self.set_pointer(left_path, common.clone());
        self.set_pointer(right_path, common);
        true
    }

    fn lookup_pointer(&self, path: &AccessPath) -> Option<PointerValue> {
        let exact = self.pointers.get(path).cloned();
        let mut value = exact.clone();
        for (candidate, candidate_value) in &self.pointers {
            if candidate == path {
                continue;
            }
            if path.matches_pattern(candidate) {
                value = Some(PointerValue::join(
                    &value.unwrap_or_else(PointerValue::bottom),
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

impl<'tcx> DomainState<'tcx> for AllocationState<'tcx> {
    fn join(left: &Self, right: &Self) -> Self {
        join_state(left, right, false)
    }

    fn widen(previous: &Self, next: &Self) -> Self {
        join_state(previous, next, true)
    }

    fn state_changed(previous: &Self, next: &Self) -> bool {
        previous.bottom != next.bottom
            || previous.pointers != next.pointers
            || previous.layouts != next.layouts
            || previous.objects != next.objects
    }
}

impl<'tcx> fmt::Display for AllocationState<'tcx> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.bottom {
            return write!(f, "Bot");
        }
        let mut entries: Vec<_> = self
            .pointers
            .iter()
            .filter(|(_, value)| !value.is_bottom())
            .map(|(path, value)| format!("{path} => {value}"))
            .collect();
        entries.extend(
            self.objects
                .iter()
                .map(|(id, fact)| format!("{id} => {fact}")),
        );
        entries.sort();
        write!(f, "{}", entries.join(", "))
    }
}

impl<'tcx> StateEntries<'tcx> for AllocationState<'tcx> {
    fn entries(&self) -> Vec<(Place<'tcx>, String)> {
        let mut entries = Vec::new();
        for (path, value) in &self.pointers {
            if let Some(place) = self.display_places.get(path) {
                entries.push((*place, value.to_string()));
            }
        }
        for (path, value) in &self.layouts {
            if let Some(place) = self.display_places.get(path) {
                entries.push((*place, format!("layout {value}")));
            }
        }
        entries
    }

    fn should_print_entry(&self, place: Place<'tcx>) -> bool {
        let Some(path) = AccessPath::from_place(place) else {
            return false;
        };
        self.pointers
            .get(&path)
            .is_some_and(|value| !value.is_bottom())
            || self
                .layouts
                .get(&path)
                .is_some_and(|value| !value.is_bottom())
    }
}

fn join_state<'tcx>(
    left: &AllocationState<'tcx>,
    right: &AllocationState<'tcx>,
    widening: bool,
) -> AllocationState<'tcx> {
    if left.bottom {
        return right.clone();
    }
    if right.bottom {
        return left.clone();
    }
    let mut out = AllocationState::empty(false, left.debug || right.debug);
    out.tracked_pointers = left
        .tracked_pointers
        .union(&right.tracked_pointers)
        .cloned()
        .collect();
    out.tracked_layouts = left
        .tracked_layouts
        .union(&right.tracked_layouts)
        .cloned()
        .collect();
    for path in &out.tracked_pointers {
        let left_value = left
            .pointers
            .get(path)
            .cloned()
            .unwrap_or_else(PointerValue::bottom);
        let right_value = right
            .pointers
            .get(path)
            .cloned()
            .unwrap_or_else(PointerValue::bottom);
        let value = if widening {
            PointerValue::widen(&left_value, &right_value)
        } else {
            PointerValue::join(&left_value, &right_value)
        };
        out.pointers.insert(path.clone(), value);
    }
    for path in &out.tracked_layouts {
        let left_value = left
            .layouts
            .get(path)
            .copied()
            .unwrap_or_else(LayoutValue::bottom);
        let right_value = right
            .layouts
            .get(path)
            .copied()
            .unwrap_or_else(LayoutValue::bottom);
        out.layouts.insert(
            path.clone(),
            if widening {
                LayoutValue::widen(left_value, right_value)
            } else {
                LayoutValue::join(left_value, right_value)
            },
        );
    }
    for id in left.objects.keys().chain(right.objects.keys()) {
        let left_fact = left
            .objects
            .get(id)
            .cloned()
            .unwrap_or_else(|| AllocationFact::absent(id.origin()));
        let right_fact = right
            .objects
            .get(id)
            .cloned()
            .unwrap_or_else(|| AllocationFact::absent(id.origin()));
        out.objects.insert(
            id.clone(),
            if widening {
                AllocationFact::widen(&left_fact, &right_fact)
            } else {
                AllocationFact::join(&left_fact, &right_fact)
            },
        );
    }
    out.display_places = join_display_places(&left.display_places, &right.display_places);
    out
}

pub fn is_allocation_pointer(tcx: TyCtxt<'_>, ty: Ty<'_>) -> bool {
    matches!(ty.kind(), TyKind::RawPtr(_, _) | TyKind::Ref(_, _, _)) || is_non_null_ty(tcx, ty)
}

pub fn is_non_null_ty(tcx: TyCtxt<'_>, ty: Ty<'_>) -> bool {
    let TyKind::Adt(def, _) = ty.kind() else {
        return false;
    };
    let path = tcx.def_path_str(def.did());
    path.ends_with("::ptr::non_null::NonNull") || path.ends_with("::ptr::NonNull")
}

pub fn pointer_pointee_type<'tcx>(tcx: TyCtxt<'tcx>, ty: Ty<'tcx>) -> Option<Ty<'tcx>> {
    match ty.kind() {
        TyKind::RawPtr(pointee, _) | TyKind::Ref(_, pointee, _) => Some(*pointee),
        TyKind::Adt(_, args) if is_non_null_ty(tcx, ty) => args.types().next(),
        _ => None,
    }
}

pub fn is_layout_ty(tcx: TyCtxt<'_>, ty: Ty<'_>) -> bool {
    let TyKind::Adt(def, _) = ty.kind() else {
        return false;
    };
    let path = tcx.def_path_str(def.did());
    path.ends_with("::alloc::layout::Layout") || path.ends_with("::alloc::Layout")
}

pub fn type_size<'tcx>(tcx: TyCtxt<'tcx>, ty: Ty<'tcx>) -> Option<Interval> {
    tcx.layout_of(TypingEnv::fully_monomorphized().as_query_input(ty))
        .ok()
        .map(|layout| {
            let bytes = layout.size.bytes() as i128;
            Interval::new(bytes, bytes)
        })
}

fn interval_is_zero(value: Interval) -> AbstractBool {
    if value.is_empty() {
        AbstractBool::Bottom
    } else if value.low == 0 && value.high == 0 {
        AbstractBool::True
    } else if value.high < 0 || value.low > 0 {
        AbstractBool::False
    } else {
        AbstractBool::Top
    }
}

fn range_within(offset: Interval, len: Interval, extent: Interval) -> AbstractBool {
    if offset.is_empty() || len.is_empty() || extent.is_empty() {
        return AbstractBool::Bottom;
    }
    let end = add(&offset, &len);
    if offset.low >= 0 && len.low >= 0 && end.high <= extent.low {
        AbstractBool::True
    } else if offset.high < 0 || len.high < 0 || end.low > extent.high {
        AbstractBool::False
    } else {
        AbstractBool::Top
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::allocation::abstract_value::AllocationSite;
    use rustc_middle::mir::Local;
    use rustc_span::DUMMY_SP;

    fn heap(index: usize) -> AllocationId {
        let local = Local::from_usize(index);
        AllocationId::Heap(AllocationSite {
            span: DUMMY_SP,
            destination: AccessPath::from_local(local),
        })
    }

    #[test]
    fn range_query_combines_liveness_offset_and_extent() {
        let id = heap(1);
        let mut state = AllocationState::bottom(false);
        state.set_object(
            id.clone(),
            AllocationFact::live(AllocationOrigin::Heap, Interval::new(16, 16)),
        );
        let valid = PointerValue::target(id.clone(), Interval::new(4, 4));
        let invalid = PointerValue::target(id, Interval::new(12, 12));
        assert_eq!(
            state.range_in_allocation(&valid, Interval::new(4, 4)),
            AbstractBool::True
        );
        assert_eq!(
            state.range_in_allocation(&invalid, Interval::new(8, 8)),
            AbstractBool::False
        );
    }

    #[test]
    fn deallocation_changes_all_alias_queries() {
        let id = heap(1);
        let mut state = AllocationState::bottom(false);
        state.set_object(
            id.clone(),
            AllocationFact::live(AllocationOrigin::Heap, Interval::new(16, 16)),
        );
        let base = PointerValue::target(id.clone(), Interval::new(0, 0));
        let alias = PointerValue::target(id, Interval::new(8, 8));
        state.deallocate(&base, false);
        assert_eq!(state.is_live(&base), AbstractBool::False);
        assert_eq!(state.is_live(&alias), AbstractBool::False);
    }

    #[test]
    fn summary_object_prevents_definite_same_allocation() {
        let id = heap(1);
        let mut fact = AllocationFact::live(AllocationOrigin::Heap, Interval::new(16, 16));
        fact.multiplicity = fact.multiplicity.after_allocation();
        let mut state = AllocationState::bottom(false);
        state.set_object(id.clone(), fact);
        let left = PointerValue::target(id.clone(), Interval::new(0, 0));
        let right = PointerValue::target(id, Interval::new(4, 4));
        assert_eq!(state.same_allocation(&left, &right), AbstractBool::Top);
    }
}
