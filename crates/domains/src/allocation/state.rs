use super::abstract_value::{
    AbstractBool, AllocationFact, AllocationId, AllocationMultiplicity, AllocationOrigin,
    AllocationStatus, LayoutValue, PointerValue,
};
use crate::interval::abstract_value::{Interval, add};
use mirsa_framework::access_path::AccessPath;
pub use mirsa_framework::call_model::is_vec_ty;
use mirsa_framework::forward::DomainState;
use mirsa_framework::printer::StateEntries;
use mirsa_framework::revision::{Revision, Versioned};
use mirsa_relations::symbolic::{SymbolicState, join_display_places};
use rustc_middle::mir::{Body, LocalDecls, Operand, Place};
use rustc_middle::ty::{Ty, TyCtxt, TyKind, TypingEnv};
use std::collections::{HashMap, HashSet};
use std::fmt;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AllocationState<'tcx> {
    pointers: Versioned<HashMap<AccessPath, PointerValue>>,
    buffers: Versioned<HashMap<AccessPath, PointerValue>>,
    layouts: Versioned<HashMap<AccessPath, LayoutValue>>,
    objects: Versioned<HashMap<AllocationId, AllocationFact>>,
    tracked_pointers: Versioned<HashSet<AccessPath>>,
    tracked_layouts: Versioned<HashSet<AccessPath>>,
    display_places: Versioned<HashMap<AccessPath, Place<'tcx>>>,
    bottom: bool,
    debug: bool,
}

pub enum AllocationEntry<'a> {
    Pointer(&'a AccessPath, &'a PointerValue),
    Buffer(&'a AccessPath, &'a PointerValue),
    Layout(&'a AccessPath, &'a LayoutValue),
    Object(&'a AllocationId, &'a AllocationFact),
}

impl<'tcx> AllocationState<'tcx> {
    pub fn export_facts(&self, roots: &[AccessPath]) -> Vec<AllocationEntry<'_>> {
        use mirsa_framework::facts::selected;
        let mut out = Vec::new();
        for (p, v) in &self.pointers {
            if selected(p, roots) {
                out.push(AllocationEntry::Pointer(p, v));
            }
        }
        for (p, v) in &self.buffers {
            if selected(p, roots) {
                out.push(AllocationEntry::Buffer(p, v));
            }
        }
        for (p, v) in &self.layouts {
            if selected(p, roots) {
                out.push(AllocationEntry::Layout(p, v));
            }
        }
        let mut ids: HashSet<_> = self
            .pointers
            .iter()
            .chain(&self.buffers)
            .filter(|(path, _)| selected(path, roots))
            .flat_map(|(_, value)| value.targets().map(|(id, _)| id))
            .collect();
        ids.extend(self.objects.keys().filter(|id| match id {
            AllocationId::Stack(local) | AllocationId::External(local) => {
                roots.iter().any(|p| p.root == *local)
            }
            AllocationId::ExternalPath(path) | AllocationId::ExternalBuffer(path) => {
                selected(path, roots)
            }
            _ => false,
        }));
        for id in ids {
            if let Some(fact) = self.objects.get(id) {
                out.push(AllocationEntry::Object(id, fact));
            }
        }
        out
    }

    pub fn project(
        &self,
        map: &impl mirsa_framework::mapping::PathMapping<'tcx>,
        objects: &impl Fn(&AllocationId) -> Option<AllocationId>,
    ) -> Self {
        let pointers = self
            .pointers
            .iter()
            .filter_map(|(path, value)| Some((map.path(path)?, value.map_objects(objects))))
            .collect();
        let buffers = self
            .buffers
            .iter()
            .filter_map(|(path, value)| Some((map.path(path)?, value.map_objects(objects))))
            .collect();
        Self {
            pointers,
            buffers,
            layouts: map.values(&self.layouts).into(),
            objects: self
                .objects
                .iter()
                .filter_map(|(id, value)| Some((objects(id)?, value.clone())))
                .collect(),
            tracked_pointers: map.paths(&self.tracked_pointers).into(),
            tracked_layouts: map.paths(&self.tracked_layouts).into(),
            display_places: map.places(&self.display_places).into(),
            bottom: self.bottom,
            debug: self.debug,
        }
    }

    pub fn extend(&mut self, other: Self) {
        self.pointers.extend(other.pointers);
        self.buffers.extend(other.buffers);
        self.layouts.extend(other.layouts);
        self.objects.extend(other.objects);
        self.tracked_pointers.extend(other.tracked_pointers);
        self.tracked_layouts.extend(other.tracked_layouts);
        self.display_places.extend(other.display_places);
        self.bottom &= other.bottom;
    }

    pub fn referenced_objects(
        &self,
        roots: &HashSet<rustc_middle::mir::Local>,
    ) -> HashSet<AllocationId> {
        self.pointers
            .iter()
            .chain(&self.buffers)
            .filter(|(path, _)| roots.contains(&path.root))
            .flat_map(|(_, value)| value.targets().map(|(id, _)| id.clone()))
            .collect()
    }

    pub fn invalidate_paths(&mut self, symbolic: &SymbolicState<'tcx>, paths: &[AccessPath]) {
        let affected: Vec<_> = self
            .buffers
            .iter()
            .filter(|(path, _)| symbolic.write_affects(path, paths))
            .map(|(path, value)| (path.clone(), value.clone()))
            .collect();
        for (path, value) in affected {
            self.deallocate(&value, true);
            self.buffers.insert(path, PointerValue::unknown_non_null());
        }
        for (path, value) in &mut self.pointers {
            if symbolic.write_affects(path, paths) {
                *value = PointerValue::top();
            }
        }
        for (path, value) in &mut self.layouts {
            if symbolic.write_affects(path, paths) {
                *value = LayoutValue::top();
            }
        }
    }

    fn empty(bottom: bool, debug: bool) -> Self {
        Self {
            pointers: HashMap::new().into(),
            buffers: HashMap::new().into(),
            layouts: HashMap::new().into(),
            objects: HashMap::new().into(),
            tracked_pointers: HashSet::new().into(),
            tracked_layouts: HashSet::new().into(),
            display_places: HashMap::new().into(),
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
            let mut fact = AllocationFact::live(AllocationOrigin::Stack, extent);
            fact.alignment = type_alignment(tcx, decl.ty);
            if !mirsa_framework::config::storage_markers_preserved(tcx)
                && local.index() > body.arg_count
            {
                fact.status = AllocationStatus::top();
            }
            state.objects.insert(AllocationId::Stack(local), fact);
        }

        for place in places {
            let local_idx = place.local.index();
            let is_arg = local_idx >= 1 && local_idx <= body.arg_count;
            let Some(path) = AccessPath::from_place(*place) else {
                continue;
            };
            let ty = place.ty(&body.local_decls, tcx).ty;
            let external_id = if place.projection.is_empty() {
                AllocationId::External(place.local)
            } else {
                AllocationId::ExternalPath(path.clone())
            };
            let (buffer_path, buffer_ty) = match ty.kind() {
                TyKind::Ref(_, inner, _) => (path.deref(), *inner),
                _ => (path.clone(), ty),
            };
            if is_arg && is_vec_ty(tcx, buffer_ty) {
                let id = AllocationId::ExternalBuffer(buffer_path.clone());
                state.set_object(
                    id.clone(),
                    AllocationFact::live(AllocationOrigin::External, Interval::new(0, i128::MAX)),
                );
                state.set_buffer(
                    buffer_path,
                    PointerValue::join(
                        &PointerValue::target(id, Interval::new(0, 0)),
                        &PointerValue::invalid_non_null(),
                    ),
                );
            }
            if state.tracked_pointers.contains(&path) {
                let value = if is_arg {
                    match ty.kind() {
                        TyKind::Ref(_, pointee, _) => {
                            let id = external_id.clone();
                            let minimum = type_size(tcx, *pointee)
                                .map(|size| Interval::new(size.low, i128::MAX))
                                .unwrap_or_else(Interval::top);
                            state.objects.insert(
                                id.clone(),
                                AllocationFact {
                                    alignment: type_alignment(tcx, *pointee),
                                    ..AllocationFact::live(AllocationOrigin::External, minimum)
                                },
                            );
                            let pointer = PointerValue::target(id, Interval::new(0, 0));
                            // Valid zero-sized references (including empty slices)
                            // may be aligned dangling pointers, without an allocation.
                            if minimum.low <= 0 {
                                PointerValue::join(&pointer, &PointerValue::invalid_non_null())
                            } else {
                                pointer
                            }
                        }
                        TyKind::RawPtr(_, _) => PointerValue::top(),
                        TyKind::Adt(_, _) if is_non_null_ty(tcx, ty) => {
                            let id = external_id.clone();
                            state.objects.insert(
                                id.clone(),
                                AllocationFact {
                                    alignment: 1,
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
                        _ => PointerValue::top(),
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
        for place in places {
            if place.local.index() == 0 || place.local.index() > body.arg_count {
                continue;
            }
            let ty = place.ty(&body.local_decls, tcx).ty;
            if !ty.is_box() {
                continue;
            }
            let Some(path) = AccessPath::from_place(*place) else {
                continue;
            };
            let id = AllocationId::ExternalBuffer(path.clone());
            let size = ty.boxed_ty().and_then(|pointee| type_size(tcx, pointee));
            let extent = size
                .map(|size| Interval::new(size.low, i128::MAX))
                .unwrap_or_else(|| Interval::new(0, i128::MAX));
            state.set_object(
                id.clone(),
                AllocationFact {
                    alignment: ty.boxed_ty().map(|ty| type_alignment(tcx, ty)).unwrap_or(1),
                    ..AllocationFact::live(AllocationOrigin::External, extent)
                },
            );
            let pointer = match size {
                Some(size) if size.high == 0 => PointerValue::invalid_non_null(),
                Some(size) if size.low > 0 => PointerValue::target(id, Interval::new(0, 0)),
                _ => PointerValue::join(
                    &PointerValue::target(id, Interval::new(0, 0)),
                    &PointerValue::invalid_non_null(),
                ),
            };
            state.set_buffer(path.clone(), pointer.clone());
            for candidate in places {
                let Some(candidate_path) = AccessPath::from_place(*candidate) else {
                    continue;
                };
                let Some(suffix) = candidate_path.strip_pattern_prefix(&path) else {
                    continue;
                };
                if !suffix.is_empty()
                    && suffix.len() <= 3
                    && suffix.iter().all(|elem| {
                        matches!(elem, mirsa_framework::access_path::AccessPathElem::Field(0))
                    })
                    && state.tracked_pointers.contains(&candidate_path)
                {
                    state.set_pointer(candidate_path, pointer.clone());
                }
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
            if is_layout_ty(tcx, ty) || is_layout_result_ty(tcx, ty) {
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
        self.display_places.insert_if_absent(path.clone(), place);
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
        self.display_places.insert_if_absent(path.clone(), place);
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

    pub(super) fn forget_call_result(
        &mut self,
        symbolic: &SymbolicState<'tcx>,
        place: Place<'tcx>,
    ) {
        let Some(path) = self.pointer_path_resolved(symbolic, place) else {
            return;
        };
        for (candidate, value) in &mut self.pointers {
            if candidate.strip_pattern_prefix(&path).is_some() {
                *value = PointerValue::top();
            }
        }
        for (candidate, value) in &mut self.layouts {
            if candidate.strip_pattern_prefix(&path).is_some() {
                *value = LayoutValue::top();
            }
        }
        self.forget_buffers(&path);
    }

    pub fn copy_pointer_tree(&mut self, dst: &AccessPath, src: &AccessPath, default: PointerValue) {
        let buffers: Vec<_> = self
            .buffers
            .iter()
            .filter_map(|(path, value)| {
                path.strip_pattern_prefix(src)
                    .map(|suffix| (dst.join_suffix(&suffix), value.clone()))
            })
            .collect();
        self.buffers
            .retain(|path, _| path.strip_pattern_prefix(dst).is_none());
        self.buffers.extend(buffers);
        for (dst_path, src_path) in
            mirsa_framework::mapping::subtree_pairs(dst, src, self.tracked_pointers.iter(), true)
        {
            if !self.tracked_pointers.contains(&dst_path) {
                continue;
            }
            let value = self
                .lookup_pointer(&src_path)
                .unwrap_or_else(|| default.clone());
            self.set_pointer(dst_path, value);
        }
    }

    pub(super) fn assign_from_operand(
        &mut self,
        symbolic: &SymbolicState<'tcx>,
        destination: Place<'tcx>,
        operand: &Operand<'tcx>,
    ) {
        let Some(source) = operand
            .place()
            .and_then(|place| self.pointer_path_resolved(symbolic, place))
        else {
            return;
        };
        if let Some(destination) = self.pointer_path_resolved(symbolic, destination) {
            self.copy_pointer_tree(&destination, &source, PointerValue::top());
            if matches!(operand, Operand::Move(_)) && destination != source {
                self.forget_buffers(&source);
            }
        } else if matches!(operand, Operand::Move(_)) {
            self.drop_buffers(&source, true);
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
        self.display_places.insert_if_absent(path, place);
    }

    pub fn object(&self, id: &AllocationId) -> Option<&AllocationFact> {
        self.objects.get(id)
    }

    pub fn buffer(&self, path: &AccessPath) -> PointerValue {
        self.buffers
            .get(path)
            .cloned()
            .unwrap_or_else(PointerValue::unknown_non_null)
    }

    pub fn set_buffer(&mut self, path: AccessPath, value: PointerValue) {
        self.bottom = false;
        self.buffers.insert(path, value);
    }

    pub fn forget_buffers(&mut self, path: &AccessPath) {
        self.buffers
            .retain(|candidate, _| candidate.strip_pattern_prefix(path).is_none());
    }

    pub fn drop_buffers(&mut self, path: &AccessPath, conditional: bool) {
        let values: Vec<_> = self
            .buffers
            .iter()
            .filter(|(candidate, _)| candidate.strip_pattern_prefix(path).is_some())
            .map(|(_, value)| value.clone())
            .collect();
        for value in values {
            self.deallocate(&value, conditional);
        }
        self.forget_buffers(path);
    }

    pub fn allocate_buffer(&mut self, id: AllocationId, extent: Interval) -> PointerValue {
        if extent.high == 0 {
            return PointerValue::invalid_non_null();
        }
        self.allocate_fallibly(id.clone(), extent);
        let mut fact = self.objects[&id].clone();
        if fact.multiplicity.is_exactly_one() {
            fact.status = AllocationStatus::live();
        }
        self.set_object(id.clone(), fact);
        let value = PointerValue::target(id, Interval::new(0, 0));
        if extent.low <= 0 {
            PointerValue::join(&value, &PointerValue::invalid_non_null())
        } else {
            value
        }
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
                alignment: 1,
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
        let aliases: Vec<_> = self
            .objects
            .keys()
            .filter(|candidate| {
                !ids.contains(candidate) && ids.iter().any(|id| id.may_alias(candidate))
            })
            .cloned()
            .collect();
        for id in aliases {
            if let Some(mut fact) = self.objects.get(&id).cloned() {
                fact.status = fact.status.join(AllocationStatus::dead());
                self.set_object(id, fact);
            }
        }
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
            if fact.status != AllocationStatus::live() {
                fact.multiplicity = fact.multiplicity.after_allocation();
            }
            fact.status.join(AllocationStatus::live())
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

    pub fn constrain_minimum_extent(&mut self, pointer: &PointerValue, bytes: i128) -> bool {
        let Some((id, offset)) = pointer.exact_target() else {
            return true;
        };
        if offset.is_empty() || offset.low < 0 || bytes <= 0 {
            return true;
        }
        let Some(minimum) = offset.low.checked_add(bytes) else {
            return true;
        };
        let Some(fact) = self.objects.get(id) else {
            return true;
        };
        if !fact.status.is_definitely_live() || !fact.multiplicity.is_exactly_one() {
            return true;
        }
        if minimum > fact.extent.high {
            return false;
        }
        if minimum > fact.extent.low {
            self.objects.get_mut(id).unwrap().extent.low = minimum;
        }
        true
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

    pub fn pointer_within_budget(
        &self,
        symbolic: &SymbolicState<'tcx>,
        place: Place<'tcx>,
        limit: usize,
    ) -> bool {
        let Some(path) = self.pointer_path_resolved(symbolic, place) else {
            return true;
        };
        let Some(patterns) = path.matching_patterns() else {
            return false;
        };
        let mut count = 0;
        for pattern in patterns {
            if let Some(value) = self.pointers.get(&pattern) {
                count += value.target_count();
                if count > limit {
                    return false;
                }
            }
        }
        true
    }

    fn lookup_pointer(&self, path: &AccessPath) -> Option<PointerValue> {
        let exact = self.pointers.get(path).cloned();
        let mut value = exact.clone();
        let Some(patterns) = path.matching_patterns() else {
            return Some(PointerValue::top());
        };
        for candidate in patterns.into_iter().skip(1) {
            if let Some(candidate_value) = self.pointers.get(&candidate) {
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
            || previous.buffers != next.buffers
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
            self.buffers
                .iter()
                .map(|(path, value)| format!("buffer({path}) => {value}")),
        );
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
    for path in left.buffers.keys().chain(right.buffers.keys()) {
        let a = left.buffer(path);
        let b = right.buffer(path);
        out.buffers.insert(
            path.clone(),
            if widening {
                PointerValue::widen(&a, &b)
            } else {
                PointerValue::join(&a, &b)
            },
        );
    }
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
    out.display_places = join_display_places(&left.display_places, &right.display_places).into();
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

pub fn is_layout_result_ty(tcx: TyCtxt<'_>, ty: Ty<'_>) -> bool {
    let TyKind::Adt(adt, args) = ty.kind() else {
        return false;
    };
    tcx.is_diagnostic_item(rustc_span::Symbol::intern("Result"), adt.did())
        && args
            .types()
            .next()
            .is_some_and(|inner| is_layout_ty(tcx, inner))
}

pub fn is_layout_ty(tcx: TyCtxt<'_>, ty: Ty<'_>) -> bool {
    let TyKind::Adt(def, _) = ty.kind() else {
        return false;
    };
    let path = tcx.def_path_str(def.did());
    path.ends_with("::alloc::layout::Layout") || path.ends_with("::alloc::Layout")
}

pub fn type_alignment<'tcx>(tcx: TyCtxt<'tcx>, ty: Ty<'tcx>) -> u64 {
    let ty = match ty.kind() {
        TyKind::Slice(element) => *element,
        TyKind::Str => return 1,
        _ => ty,
    };
    tcx.layout_of(TypingEnv::fully_monomorphized().as_query_input(ty))
        .ok()
        .map(|layout| layout.align.abi.bytes())
        .unwrap_or(1)
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

impl<'tcx> AllocationState<'tcx> {
    pub fn revision(&self) -> ([Revision; 7], bool) {
        (
            [
                self.pointers.revision(),
                self.buffers.revision(),
                self.layouts.revision(),
                self.objects.revision(),
                self.tracked_pointers.revision(),
                self.tracked_layouts.revision(),
                self.display_places.revision(),
            ],
            self.bottom,
        )
    }
}
