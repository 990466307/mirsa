use super::SymbolicState;
use mirsa_framework::access_path::{AccessPath, AccessPathElem};
use rustc_middle::mir::{
    LocalDecls, Operand, Place, Statement, StatementKind, Terminator, TerminatorKind,
};
use rustc_middle::ty::{Ty, TyCtxt, TyKind, TypingEnv};
use std::collections::HashSet;

impl<'tcx> SymbolicState<'tcx> {
    fn possible_pointees(&self, pointer: &AccessPath) -> HashSet<AccessPath> {
        let targets: HashSet<_> = self
            .points_to
            .iter()
            .filter(|(candidate, _)| {
                self.eq
                    .equiv_readonly((*candidate).clone(), pointer.clone())
            })
            .map(|(_, pointee)| self.normalize_path(pointee))
            .collect();
        if !targets.is_empty()
            && targets.iter().all(|path| {
                !path
                    .elems
                    .contains(&mirsa_framework::access_path::AccessPathElem::Deref)
            })
        {
            return targets;
        }
        // A reborrow may name an unresolved dereference after a join. Knowing
        // that symbolic location does not identify its concrete pointee.
        let mut targets = targets;
        targets.extend(self.address_taken.iter().cloned());
        targets.insert(self.normalize_path(&pointer.deref()));
        targets.extend(
            self.display_places
                .keys()
                .filter(|path| {
                    path.elems
                        .contains(&mirsa_framework::access_path::AccessPathElem::Deref)
                })
                .cloned(),
        );
        targets
    }

    pub fn call_write_paths(
        &self,
        tcx: TyCtxt<'tcx>,
        term: &Terminator<'tcx>,
        local_decls: &LocalDecls<'tcx>,
    ) -> Vec<AccessPath> {
        if let TerminatorKind::Drop { place, .. } = &term.kind {
            let ty = place.ty(local_decls, tcx).ty;
            if !ty.needs_drop(tcx, TypingEnv::fully_monomorphized()) {
                return Vec::new();
            }
            let mut paths = self.escaped.clone();
            if let Some(path) = self.normalize_place(*place) {
                paths.insert(path.clone());
                self.collect_value_writes(tcx, path, ty, &mut paths);
            }
            return paths.into_iter().collect();
        }
        let TerminatorKind::Call { func, args, .. } = &term.kind else {
            return Vec::new();
        };
        if let Some(call) =
            mirsa_framework::call_model::RangeIteratorCall::classify(tcx, local_decls, term)
        {
            return match call {
                mirsa_framework::call_model::RangeIteratorCall::Identity(_) => Vec::new(),
                mirsa_framework::call_model::RangeIteratorCall::Next { receiver, .. } => {
                    let Some((rustc_middle::mir::ProjectionElem::Deref, prefix)) =
                        receiver.projection.split_last()
                    else {
                        return self.address_taken.iter().cloned().collect();
                    };
                    let pointer = Place {
                        local: receiver.local,
                        projection: tcx.mk_place_elems(prefix),
                    };
                    self.normalize_place(pointer)
                        .map(|path| self.possible_pointees(&path))
                        .unwrap_or_else(|| (*self.address_taken).clone())
                        .into_iter()
                        .collect()
                }
            };
        }
        if mirsa_framework::call_model::BufferCall::classify(tcx, func, local_decls)
            .preserves_memory()
        {
            return Vec::new();
        }
        if let TyKind::FnDef(callee, _) = func.ty(local_decls, tcx).kind() {
            if mirsa_framework::call_model::ValueQuery::classify(tcx, *callee).is_some()
                || mirsa_framework::call_model::PointerOffset::classify(tcx, func, local_decls)
                    .is_some()
                || mirsa_framework::call_model::PointerView::classify(tcx, func, local_decls)
                    .is_some()
            {
                return Vec::new();
            }
        }
        let mut paths = self.escaped.clone();
        for arg in args {
            if let Operand::Copy(place) | Operand::Move(place) = &arg.node {
                if let Some(path) = self.normalize_place(*place) {
                    self.collect_value_writes(tcx, path, arg.node.ty(local_decls, tcx), &mut paths);
                }
            }
        }
        loop {
            let before = paths.len();
            for (pointer, pointee) in &self.points_to {
                if paths
                    .iter()
                    .any(|path| pointer.strip_pattern_prefix(path).is_some())
                {
                    paths.insert(pointee.clone());
                }
            }
            if paths.len() == before {
                break;
            }
        }
        paths.into_iter().collect()
    }

    // References can be carried inside values (including closure captures).
    // Exhausting the traversal budget must forget memory, never keep old facts.
    fn collect_value_writes(
        &self,
        tcx: TyCtxt<'tcx>,
        root: AccessPath,
        ty: Ty<'tcx>,
        paths: &mut HashSet<AccessPath>,
    ) {
        const MAX_VALUES: usize = 128;
        let mut pending = vec![(root, ty)];
        let mut visited = HashSet::new();
        while let Some((path, ty)) = pending.pop() {
            if !visited.insert((path.clone(), ty)) {
                continue;
            }
            if visited.len() > MAX_VALUES || pending.len() > MAX_VALUES {
                paths.extend(self.address_taken.iter().cloned());
                paths.extend(
                    self.display_places
                        .keys()
                        .filter(|p| p.elems.contains(&AccessPathElem::Deref))
                        .cloned(),
                );
                return;
            }
            match ty.kind() {
                TyKind::Ref(_, inner, mutability) => {
                    if mutability.is_mut()
                        || !inner.is_freeze(tcx, TypingEnv::fully_monomorphized())
                    {
                        paths.extend(self.possible_pointees(&path));
                    }
                    pending.push((self.normalize_path(&path.deref()), *inner));
                }
                TyKind::RawPtr(inner, _) => {
                    paths.extend(self.possible_pointees(&path));
                    pending.push((self.normalize_path(&path.deref()), *inner));
                }
                TyKind::Array(inner, len) => {
                    if let Some(len) = len
                        .try_to_target_usize(tcx)
                        .filter(|n| *n <= MAX_VALUES as u64)
                    {
                        pending.extend((0..len).map(|offset| {
                            (
                                path.join_suffix(&[AccessPathElem::ConstantIndex {
                                    offset,
                                    from_end: false,
                                }]),
                                *inner,
                            )
                        }));
                    } else {
                        pending.push((path.join_suffix(&[AccessPathElem::Index]), *inner));
                    }
                }
                TyKind::Slice(inner) => {
                    pending.push((path.join_suffix(&[AccessPathElem::Index]), *inner))
                }
                TyKind::Dynamic(..) | TyKind::Param(_) | TyKind::Alias(..) => {
                    paths.extend(self.address_taken.iter().cloned());
                    paths.insert(path);
                }
                _ => path.for_each_field(tcx, ty, |field, ty| pending.push((field, ty))),
            }
        }
    }

    pub fn indirect_write_paths(&self, stmt: &Statement<'tcx>) -> Vec<AccessPath> {
        let StatementKind::Assign(assign) = &stmt.kind else {
            return Vec::new();
        };
        let Some(path) = self.normalize_place(assign.0) else {
            return Vec::new();
        };
        if !path
            .elems
            .contains(&mirsa_framework::access_path::AccessPathElem::Deref)
        {
            return Vec::new();
        }
        let mut paths = self.address_taken.clone();
        paths.insert(path);
        paths.into_iter().collect()
    }
}
