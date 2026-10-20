use mirsa_framework::revision::{Revision, Versioned};
mod comparison;
mod effects;
mod facts;
pub use facts::SymbolicEntry;
mod index;
mod join;
mod predicate;
mod projection;
pub use predicate::PredicateFact;

use crate::value_expr::ValueExpr;
use mirsa_framework::access_path::AccessPath;
use mirsa_framework::eq_domain::EqDomain;
use mirsa_framework::reduction::MAX_RELATED;
use rustc_hir::def_id::DefId;
use rustc_middle::mir::{
    BinOp, CastKind, LocalDecls, Operand, Place, Rvalue, Statement, StatementKind, Terminator,
    TerminatorKind,
};
use rustc_middle::ty::{TyCtxt, TyKind};
use std::collections::{HashMap, HashSet};
use std::fmt;

#[derive(Clone, Debug, PartialEq)]
pub enum SymbolicExpr<'tcx> {
    Pointer {
        base: Operand<'tcx>,
        offset: Option<(mirsa_framework::call_model::PointerOffset, Operand<'tcx>)>,
    },
    PointerFromAddress {
        address: Operand<'tcx>,
    },
    Value(ValueExpr<'tcx>),
    Cmp {
        op: BinOp,
        left: Operand<'tcx>,
        right: Operand<'tcx>,
    },
    Address {
        pointer: Operand<'tcx>,
    },
    Length {
        place: Place<'tcx>,
    },
    Call {
        callee: DefId,
        args: Vec<Operand<'tcx>>,
    },
}

impl<'tcx> SymbolicExpr<'tcx> {
    pub fn visit_places(&self, mut visit: impl FnMut(Place<'tcx>)) {
        let mut operand = |operand: &Operand<'tcx>| {
            if let Some(place) = operand.place() {
                visit(place);
            }
        };
        match self {
            Self::Pointer { base, offset } => {
                operand(base);
                if let Some((_, count)) = offset {
                    operand(count);
                }
            }
            Self::PointerFromAddress { address } => operand(address),
            Self::Value(value) => value.visit_operands(operand),
            Self::Cmp { left, right, .. } => {
                operand(left);
                operand(right);
            }
            Self::Address { pointer } => operand(pointer),
            Self::Length { place } => visit(*place),
            Self::Call { args, .. } => {
                for arg in args {
                    operand(arg);
                }
            }
        }
    }
}

impl<'tcx> Eq for SymbolicExpr<'tcx> {}

#[derive(Clone, Debug, PartialEq, Hash)]
pub enum SymbolicFact<'tcx> {
    EqConst { expr: Operand<'tcx>, value: u128 },
    NeConst { expr: Operand<'tcx>, value: u128 },
}

impl<'tcx> Eq for SymbolicFact<'tcx> {}

#[derive(Clone, Debug)]
pub struct SymbolicState<'tcx> {
    pub eq: EqDomain<'tcx, AccessPath>,
    display_places: Versioned<HashMap<AccessPath, Place<'tcx>>>,
    exprs: Versioned<HashMap<AccessPath, SymbolicExpr<'tcx>>>,
    facts: Versioned<Vec<SymbolicFact<'tcx>>>,
    predicates: Versioned<Vec<PredicateFact<'tcx>>>,
    predicate_index: Versioned<HashMap<AccessPath, Vec<usize>>>,
    dependents: Versioned<HashMap<AccessPath, Vec<AccessPath>>>,
    dependencies: Versioned<HashMap<AccessPath, Vec<AccessPath>>>,
    fact_index: Versioned<HashMap<AccessPath, Vec<usize>>>,
    points_to: Versioned<HashMap<AccessPath, AccessPath>>,
    address_taken: Versioned<HashSet<AccessPath>>,
    escaped: Versioned<HashSet<AccessPath>>,
    index_inputs: index::IndexInputs,
    debug: bool,
}

impl PartialEq for SymbolicState<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.eq == other.eq
            && self.display_places == other.display_places
            && self.exprs == other.exprs
            && self.facts == other.facts
            && self.predicates == other.predicates
            && self.points_to == other.points_to
            && self.address_taken == other.address_taken
            && self.escaped == other.escaped
            && self.debug == other.debug
    }
}

impl<'tcx> Eq for SymbolicState<'tcx> {}

impl<'tcx> SymbolicState<'tcx> {
    pub fn new() -> Self {
        Self {
            eq: EqDomain::new(),
            display_places: HashMap::new().into(),
            exprs: HashMap::new().into(),
            facts: Vec::new().into(),
            predicates: Vec::new().into(),
            predicate_index: HashMap::new().into(),
            dependents: HashMap::new().into(),
            dependencies: HashMap::new().into(),
            fact_index: HashMap::new().into(),
            points_to: HashMap::new().into(),
            address_taken: HashSet::new().into(),
            escaped: HashSet::new().into(),
            index_inputs: Default::default(),
            debug: false,
        }
    }

    pub fn new_with_debug(debug: bool) -> Self {
        let mut out = Self::new();
        out.debug = debug;
        out
    }

    pub fn debug(&self, args: fmt::Arguments<'_>) {
        if self.debug {
            eprintln!("[symbolic] {args}");
        }
    }

    pub fn remember_place(&mut self, path: AccessPath, place: Place<'tcx>) {
        self.display_places.insert(path, place);
    }

    pub fn remember_places(&mut self, places: impl IntoIterator<Item = (AccessPath, Place<'tcx>)>) {
        for (path, place) in places {
            self.remember_place(path, place);
        }
    }

    pub fn kill_place(&mut self, place: Place<'tcx>) {
        if let Some(path) = AccessPath::from_place(place) {
            self.eq.kill(path.clone());
            self.remove_expr(&path);
            self.points_to.remove(&path);
        }
    }

    pub fn kill_place_tree(&mut self, place: Place<'tcx>) {
        let Some(path) = AccessPath::from_place(place) else {
            return;
        };
        self.kill_path_tree(&path);
    }

    pub fn kill_path_tree(&mut self, path: &AccessPath) {
        let mut affected: HashSet<AccessPath> = HashSet::from([path.clone()]);
        for candidate in self.display_places.keys().chain(self.points_to.keys()) {
            if self.write_affects(candidate, std::slice::from_ref(path)) {
                affected.insert(candidate.clone());
            }
        }
        let removed: Vec<_> = self
            .exprs
            .iter()
            .filter(|(_, expr)| {
                affected
                    .iter()
                    .any(|path| expr_mentions_path_tree(expr, path))
            })
            .map(|(path, _)| path.clone())
            .collect();
        for path in removed {
            self.remove_expr(&path);
        }
        self.predicates.retain(|fact| {
            !fact.values.values().any(|operand| {
                affected
                    .iter()
                    .any(|path| operand_mentions_path_tree(operand, path))
            })
        });
        self.facts.retain(|fact| {
            !affected
                .iter()
                .any(|path| fact_mentions_path_tree(fact, path))
        });
        self.rebuild_fact_index();
        for affected_path in affected {
            self.eq.kill(affected_path.clone());
            self.remove_expr(&affected_path);
            self.points_to.remove(&affected_path);
            self.debug(format_args!("kill {affected_path}"));
        }
    }

    pub fn write_affects(&self, candidate: &AccessPath, written: &[AccessPath]) -> bool {
        let normalized = self.normalize_path(candidate);
        written.iter().any(|path| {
            normalized.strip_pattern_prefix(path).is_some()
                || path
                    .strip_pattern_prefix(&normalized)
                    .is_some_and(|suffix| {
                        !suffix.contains(&mirsa_framework::access_path::AccessPathElem::Deref)
                    })
        })
    }

    pub fn set_points_to(&mut self, pointer: AccessPath, pointee: AccessPath) {
        self.address_taken.insert(pointee.clone());
        self.debug(format_args!("points_to {pointer} -> {pointee}"));
        self.points_to.insert(pointer, pointee);
    }

    pub fn copy_points_to(&mut self, dst: AccessPath, src: &AccessPath) {
        let copied: Vec<_> = self
            .points_to
            .iter()
            .filter_map(|(path, pointee)| {
                let suffix = path.value_suffix(src)?;
                Some((dst.join_suffix(suffix), pointee.clone()))
            })
            .collect();
        for (pointer, pointee) in copied {
            self.set_points_to(pointer, pointee);
        }
    }

    pub fn normalize_path(&self, path: &AccessPath) -> AccessPath {
        let mut out = AccessPath::from_local(path.root);
        for elem in &path.elems {
            match elem {
                mirsa_framework::access_path::AccessPathElem::Deref => {
                    if let Some(target) = self.points_to.get(&out) {
                        out = target.clone();
                    } else {
                        out = out.deref();
                    }
                }
                _ => out = out.join_suffix(std::slice::from_ref(elem)),
            }
        }
        out
    }

    pub fn normalize_place(&self, place: Place<'tcx>) -> Option<AccessPath> {
        let path = AccessPath::from_place(place)?;
        Some(self.normalize_path(&path))
    }

    fn place_for_path(&self, path: &AccessPath) -> Option<Place<'tcx>> {
        if path.elems.is_empty() {
            return Some(Place::from(path.root));
        }
        let place = *self.display_places.get(path)?;
        // A saved dereference may name a different object after rebinding.
        (self.normalize_place(place).as_ref() == Some(path)).then_some(place)
    }

    pub fn union_places(&mut self, left: Place<'tcx>, right: Place<'tcx>) {
        let (Some(left_path), Some(right_path)) =
            (AccessPath::from_place(left), AccessPath::from_place(right))
        else {
            return;
        };
        let left_path = self.normalize_path(&left_path);
        let right_path = self.normalize_path(&right_path);
        self.debug(format_args!("eq {left_path} == {right_path}"));
        self.remember_place(left_path.clone(), left);
        self.remember_place(right_path.clone(), right);
        self.eq.union(left_path, right_path);
    }

    pub fn equiv_places_readonly(&self, left: Place<'tcx>, right: Place<'tcx>) -> bool {
        let (Some(left_path), Some(right_path)) =
            (AccessPath::from_place(left), AccessPath::from_place(right))
        else {
            return false;
        };
        self.eq.equiv_readonly(
            self.normalize_path(&left_path),
            self.normalize_path(&right_path),
        )
    }

    pub fn merge_display_places_from(&mut self, other: &Self) {
        for (path, place) in &other.display_places {
            self.display_places.insert_if_absent(path.clone(), *place);
        }
    }

    pub fn assume_eq_const(&mut self, expr: Operand<'tcx>, value: u128) {
        self.push_fact(SymbolicFact::EqConst { expr, value });
    }

    pub fn assume_ne_const(&mut self, expr: Operand<'tcx>, value: u128) {
        self.push_fact(SymbolicFact::NeConst { expr, value });
    }

    pub fn facts(&self) -> &[SymbolicFact<'tcx>] {
        &self.facts
    }

    fn value_assignments(
        &self,
        tcx: TyCtxt<'tcx>,
        local_decls: &LocalDecls<'tcx>,
        destination: Place<'tcx>,
        rvalue: &Rvalue<'tcx>,
    ) -> Vec<(Place<'tcx>, ValueExpr<'tcx>)> {
        if has_runtime_index(destination) {
            return Vec::new();
        }
        let Some(written) = self.normalize_place(destination) else {
            return Vec::new();
        };
        let mut values = crate::value_expr::assignments(tcx, local_decls, destination, rvalue);
        values.retain(|(_, value)| {
            let mut affected = false;
            value.visit_operands(|operand| {
                if let Some(place) = operand.place() {
                    affected |= has_runtime_index(place)
                        || AccessPath::from_place(place).is_none_or(|path| {
                            self.write_affects(&path, std::slice::from_ref(&written))
                        });
                }
            });
            !affected
        });
        values
    }

    pub fn set_place_expr(&mut self, place: Place<'tcx>, expr: SymbolicExpr<'tcx>) {
        let Some(path) = AccessPath::from_place(place) else {
            return;
        };
        let path = self.normalize_path(&path);
        self.debug(format_args!("expr {path} ({place:?}) := {expr:?}"));
        self.remove_expr(&path);
        self.index_expr(&path, &expr);
        self.exprs.insert(path.clone(), expr);
        self.display_places.insert(path, place);
    }

    fn expression_path(&self, path: &AccessPath) -> Option<AccessPath> {
        if self.exprs.contains_key(path) {
            return Some(path.clone());
        }
        self.eq
            .members(path.clone())
            .take(MAX_RELATED)
            .find(|path| self.exprs.contains_key(path))
    }

    pub fn expr_for_place(&self, place: Place<'tcx>) -> Option<&SymbolicExpr<'tcx>> {
        let path = self.normalize_place(place)?;
        self.exprs.get(&self.expression_path(&path)?)
    }

    pub fn expressions(&self) -> impl Iterator<Item = (Place<'tcx>, &SymbolicExpr<'tcx>)> {
        self.exprs
            .iter()
            .filter_map(|(path, expr)| self.place_for_path(path).map(|place| (place, expr)))
    }

    fn push_fact(&mut self, fact: SymbolicFact<'tcx>) {
        if !self.facts.contains(&fact) {
            self.debug(format_args!("fact {fact:?}"));
            self.index_fact(self.facts.len(), &fact);
            self.facts.push(fact);
        }
    }
}

fn operand_mentions_path_tree<'tcx>(operand: &Operand<'tcx>, path: &AccessPath) -> bool {
    let (Operand::Copy(place) | Operand::Move(place)) = operand else {
        return false;
    };
    AccessPath::from_place(*place).is_some_and(|operand_path| {
        operand_path.strip_pattern_prefix(path).is_some()
            || path
                .strip_pattern_prefix(&operand_path)
                .is_some_and(|suffix| {
                    !suffix.contains(&mirsa_framework::access_path::AccessPathElem::Deref)
                })
    })
}

fn expr_mentions_path_tree<'tcx>(expr: &SymbolicExpr<'tcx>, path: &AccessPath) -> bool {
    let mut mentions = false;
    expr.visit_places(|place| mentions |= operand_mentions_path_tree(&Operand::Copy(place), path));
    mentions
}

fn fact_mentions_path_tree<'tcx>(fact: &SymbolicFact<'tcx>, path: &AccessPath) -> bool {
    match fact {
        SymbolicFact::EqConst { expr, .. } | SymbolicFact::NeConst { expr, .. } => {
            operand_mentions_path_tree(expr, path)
        }
    }
}

fn is_cmp_op(op: BinOp) -> bool {
    matches!(
        op,
        BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge | BinOp::Eq | BinOp::Ne
    )
}

fn has_runtime_index<'tcx>(place: Place<'tcx>) -> bool {
    place
        .projection
        .iter()
        .any(|elem| matches!(elem, rustc_middle::mir::ProjectionElem::Index(_)))
}

pub fn transfer_stmt<'tcx>(
    tcx: TyCtxt<'tcx>,
    symbolic: &mut SymbolicState<'tcx>,
    stmt: &Statement<'tcx>,
    local_decls: &LocalDecls<'tcx>,
) {
    let StatementKind::Assign(assign) = &stmt.kind else {
        return;
    };
    let (dst, rvalue) = &**assign;
    let copied_predicates = match rvalue {
        Rvalue::Use(Operand::Copy(src) | Operand::Move(src))
            if src.ty(local_decls, tcx).ty == dst.ty(local_decls, tcx).ty =>
        {
            symbolic.copied_predicates(*dst, *src)
        }
        _ => Vec::new(),
    };
    let values = symbolic.value_assignments(tcx, local_decls, *dst, rvalue);
    let pointer_cast = match rvalue {
        Rvalue::Cast(CastKind::PtrToPtr, base, _) => Some(SymbolicExpr::Pointer {
            base: base.clone(),
            offset: None,
        }),
        Rvalue::Cast(CastKind::PointerWithExposedProvenance, address, _) => {
            Some(SymbolicExpr::PointerFromAddress {
                address: address.clone(),
            })
        }
        _ => None,
    }
    .filter(|expr| {
        let Some(written) = symbolic.normalize_place(*dst) else {
            return false;
        };
        let mut valid = !has_runtime_index(*dst);
        expr.visit_places(|place| {
            valid &= !has_runtime_index(place)
                && AccessPath::from_place(place).is_some_and(|path| {
                    !symbolic.write_affects(&path, std::slice::from_ref(&written))
                });
        });
        valid
    });

    if let Some(dst_path) = AccessPath::from_place(*dst) {
        let normalized = symbolic.normalize_path(&dst_path);
        symbolic.kill_path_tree(&normalized);
        if normalized != dst_path {
            symbolic.remove_expr(&dst_path);
        }
    }
    for fact in copied_predicates {
        symbolic.add_predicate(fact);
    }
    for (place, value) in values {
        if let ValueExpr::Use(operand) = &value {
            if let Some(source) = operand.place() {
                symbolic.union_places(place, source);
                if let (Some(dst), Some(src)) = (
                    symbolic.normalize_place(place),
                    symbolic.normalize_place(source),
                ) {
                    symbolic.copy_points_to(dst, &src);
                }
            }
        }
        symbolic.set_place_expr(place, SymbolicExpr::Value(value));
    }
    if let Some(expr) = pointer_cast {
        symbolic.set_place_expr(*dst, expr);
    }
    match rvalue {
        Rvalue::Use(Operand::Copy(src) | Operand::Move(src)) => {
            if has_runtime_index(*src) && !has_runtime_index(*dst) {
                return;
            }
            let expr = symbolic.expr_for_place(*src).cloned();
            symbolic.union_places(*dst, *src);
            if let (Some(dst_path), Some(src_path)) =
                (AccessPath::from_place(*dst), AccessPath::from_place(*src))
            {
                let dst_path = symbolic.normalize_path(&dst_path);
                let src_path = symbolic.normalize_path(&src_path);
                symbolic.copy_points_to(dst_path, &src_path);
            }
            if let Some(expr) = expr {
                symbolic.set_place_expr(*dst, expr);
            }
        }
        Rvalue::BinaryOp(op, ops) if is_cmp_op(*op) => {
            let (left, right) = &**ops;
            symbolic.set_place_expr(
                *dst,
                SymbolicExpr::Cmp {
                    op: *op,
                    left: left.clone(),
                    right: right.clone(),
                },
            );
        }
        Rvalue::Cast(CastKind::PointerExposeProvenance, pointer, ty)
            if matches!(ty.kind(), TyKind::Uint(rustc_middle::ty::UintTy::Usize)) =>
        {
            symbolic.set_place_expr(
                *dst,
                SymbolicExpr::Address {
                    pointer: pointer.clone(),
                },
            );
        }
        Rvalue::Len(place) => {
            symbolic.set_place_expr(*dst, SymbolicExpr::Length { place: *place });
        }
        Rvalue::UnaryOp(rustc_middle::mir::UnOp::PtrMetadata, operand)
            if matches!(
                dst.ty(local_decls, tcx).ty.kind(),
                TyKind::Uint(rustc_middle::ty::UintTy::Usize)
            ) =>
        {
            if let Some(place) = operand.place() {
                symbolic.set_place_expr(*dst, SymbolicExpr::Length { place });
            }
        }
        Rvalue::Cast(
            CastKind::PointerCoercion(_, _),
            Operand::Copy(src) | Operand::Move(src),
            _,
        ) => {
            if let (Some(dst_path), Some(src_path)) =
                (AccessPath::from_place(*dst), AccessPath::from_place(*src))
            {
                let dst_path = symbolic.normalize_path(&dst_path);
                let src_path = symbolic.normalize_path(&src_path);
                symbolic.copy_points_to(dst_path, &src_path);
            }
        }
        Rvalue::Ref(_, _, borrowed_place) | Rvalue::RawPtr(_, borrowed_place) => {
            if let Some((rustc_middle::mir::ProjectionElem::Deref, prefix)) =
                borrowed_place.projection.split_last()
            {
                let source = Place {
                    local: borrowed_place.local,
                    projection: tcx.mk_place_elems(prefix),
                };
                let pointee =
                    |place: Place<'tcx>| place.ty(local_decls, tcx).ty.builtin_deref(true);
                if pointee(source).is_some()
                    && pointee(source) == pointee(*dst)
                    && !has_runtime_index(source)
                    && !has_runtime_index(*dst)
                {
                    symbolic.union_places(*dst, source);
                }
            }
            if let (Some(dst_path), Some(src_path)) = (
                AccessPath::from_place(*dst),
                AccessPath::from_place(*borrowed_place),
            ) {
                let dst_path = symbolic.normalize_path(&dst_path);
                symbolic.set_points_to(dst_path, symbolic.normalize_path(&src_path));
            }
        }
        _ => {}
    }
}

fn pointer_call_expr<'tcx>(
    tcx: TyCtxt<'tcx>,
    term: &Terminator<'tcx>,
    local_decls: &LocalDecls<'tcx>,
) -> Option<(Place<'tcx>, SymbolicExpr<'tcx>)> {
    use mirsa_framework::call_model::{PointerOffset, PointerView};
    let TerminatorKind::Call {
        func,
        args,
        destination,
        ..
    } = &term.kind
    else {
        return None;
    };
    let argument = &args.first()?.node;
    let expr = if let Some(offset) = PointerOffset::classify(tcx, func, local_decls) {
        SymbolicExpr::Pointer {
            base: argument.clone(),
            offset: Some((offset, args.get(1)?.node.clone())),
        }
    } else {
        let view = PointerView::classify(tcx, func, local_decls)?;
        SymbolicExpr::Pointer {
            base: view.source(tcx, local_decls, argument)?,
            offset: None,
        }
    };
    Some((*destination, expr))
}

/// Apply result relations after the caller invalidates `written` in every domain.
pub fn transfer_terminator<'tcx>(
    tcx: TyCtxt<'tcx>,
    symbolic: &mut SymbolicState<'tcx>,
    term: &Terminator<'tcx>,
    local_decls: &LocalDecls<'tcx>,
    written: &[AccessPath],
) {
    let pointer_expr = pointer_call_expr(tcx, term, local_decls);
    let pointer_expr = pointer_expr.filter(|(destination, expr)| {
        let mut writes = written.to_vec();
        if let Some(path) = symbolic.normalize_place(*destination) {
            writes.push(path);
        }
        let mut survives = !has_runtime_index(*destination);
        expr.visit_places(|place| {
            survives &= !has_runtime_index(place)
                && AccessPath::from_place(place)
                    .is_some_and(|path| !symbolic.write_affects(&path, &writes));
        });
        survives
    });
    if mirsa_framework::call_model::RangeIteratorCall::classify(tcx, local_decls, term).is_none() {
        symbolic.escaped.extend(written.iter().cloned());
    }
    let TerminatorKind::Call {
        func,
        args,
        destination,
        ..
    } = &term.kind
    else {
        return;
    };
    symbolic.kill_place_tree(*destination);
    if let Some((destination, expr)) = pointer_expr {
        symbolic.set_place_expr(destination, expr);
    }

    let TyKind::FnDef(def_id, _) = func.ty(local_decls, tcx).kind() else {
        return;
    };
    if let Some(arg) = args.first() {
        if let Some(view) =
            mirsa_framework::call_model::PointerView::classify(tcx, func, local_decls)
        {
            if let Some(source) = view.source(tcx, local_decls, &arg.node) {
                let pointee = |ty: rustc_middle::ty::Ty<'tcx>| match ty.kind() {
                    TyKind::Ref(_, inner, _) | TyKind::RawPtr(inner, _) => Some(*inner),
                    TyKind::Adt(adt, args)
                        if tcx.is_diagnostic_item(
                            rustc_span::Symbol::intern("NonNull"),
                            adt.did(),
                        ) =>
                    {
                        Some(args.type_at(0))
                    }
                    _ => None,
                };
                if pointee(source.ty(local_decls, tcx)).is_some_and(|inner| {
                    Some(inner) == pointee(destination.ty(local_decls, tcx).ty)
                }) {
                    if let (Some(dst), Some(src)) = (
                        AccessPath::from_place(*destination),
                        source.place().and_then(AccessPath::from_place),
                    ) {
                        let dst = symbolic.normalize_path(&dst);
                        let src = symbolic.normalize_path(&src);
                        symbolic.copy_points_to(dst, &src);
                    }
                }
            }
        }
        use mirsa_framework::call_model::ValueQuery;
        match ValueQuery::classify(tcx, *def_id) {
            Some(ValueQuery::Length) => {
                if let Some(place) = arg.node.place() {
                    symbolic.set_place_expr(*destination, SymbolicExpr::Length { place });
                }
            }
            Some(ValueQuery::Address) => {
                symbolic.set_place_expr(
                    *destination,
                    SymbolicExpr::Address {
                        pointer: arg.node.clone(),
                    },
                );
            }
            _ => {}
        }
    }
    if matches!(destination.ty(local_decls, tcx).ty.kind(), TyKind::Bool) {
        symbolic.set_place_expr(
            *destination,
            SymbolicExpr::Call {
                callee: *def_id,
                args: args.iter().map(|arg| arg.node.clone()).collect(),
            },
        );
    }
}

pub fn join_display_places<'tcx>(
    left: &HashMap<AccessPath, Place<'tcx>>,
    right: &HashMap<AccessPath, Place<'tcx>>,
) -> HashMap<AccessPath, Place<'tcx>> {
    let mut out = HashMap::new();
    for key in left.keys().chain(right.keys()) {
        if let Some(place) = left.get(key).or_else(|| right.get(key)) {
            out.insert(key.clone(), *place);
        }
    }
    out
}

impl<'tcx> Default for SymbolicState<'tcx> {
    fn default() -> Self {
        Self::new()
    }
}

impl<'tcx> SymbolicState<'tcx> {
    pub fn revision(&self) -> [Revision; 13] {
        [
            self.eq.revision()[0],
            self.eq.revision()[1],
            self.display_places.revision(),
            self.exprs.revision(),
            self.facts.revision(),
            self.predicates.revision(),
            self.predicate_index.revision(),
            self.dependents.revision(),
            self.dependencies.revision(),
            self.fact_index.revision(),
            self.points_to.revision(),
            self.address_taken.revision(),
            self.escaped.revision(),
        ]
    }
}
