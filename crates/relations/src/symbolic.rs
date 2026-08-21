use mirsa_framework::access_path::AccessPath;
use mirsa_framework::eq_domain::{EqDomain, join_eq};
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
    Cmp {
        op: BinOp,
        left: Operand<'tcx>,
        right: Operand<'tcx>,
    },
    Call {
        callee: DefId,
        args: Vec<Operand<'tcx>>,
    },
}

impl<'tcx> Eq for SymbolicExpr<'tcx> {}

#[derive(Clone, Debug, PartialEq)]
pub enum SymbolicFact<'tcx> {
    EqConst { expr: Operand<'tcx>, value: u128 },
    NeConst { expr: Operand<'tcx>, value: u128 },
}

impl<'tcx> Eq for SymbolicFact<'tcx> {}

#[derive(Clone, Debug, PartialEq)]
pub struct SymbolicState<'tcx> {
    pub eq: EqDomain<'tcx, AccessPath>,
    display_places: HashMap<AccessPath, Place<'tcx>>,
    exprs: HashMap<AccessPath, SymbolicExpr<'tcx>>,
    facts: Vec<SymbolicFact<'tcx>>,
    points_to: HashMap<AccessPath, AccessPath>,
    debug: bool,
}

impl<'tcx> Eq for SymbolicState<'tcx> {}

impl<'tcx> SymbolicState<'tcx> {
    pub fn new() -> Self {
        Self {
            eq: EqDomain::new(),
            display_places: HashMap::new(),
            exprs: HashMap::new(),
            facts: Vec::new(),
            points_to: HashMap::new(),
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
            self.exprs.remove(&path);
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
        for candidate in self.display_places.keys() {
            if candidate.strip_pattern_prefix(path).is_some() {
                affected.insert(candidate.clone());
            }
        }
        for affected_path in affected {
            self.eq.kill(affected_path.clone());
            self.exprs.remove(&affected_path);
            self.points_to.remove(&affected_path);
            self.debug(format_args!("kill {affected_path}"));
        }
        self.exprs
            .retain(|_, expr| !expr_mentions_path_tree(expr, path));
        self.facts
            .retain(|fact| !fact_mentions_path_tree(fact, path));
    }

    pub fn set_points_to(&mut self, pointer: AccessPath, pointee: AccessPath) {
        self.debug(format_args!("points_to {pointer} -> {pointee}"));
        self.points_to.insert(pointer, pointee);
    }

    pub fn copy_points_to(&mut self, dst: AccessPath, src: &AccessPath) {
        if let Some(pointee) = self.points_to.get(src).cloned() {
            self.debug(format_args!("points_to {dst} -> {pointee}"));
            self.points_to.insert(dst, pointee);
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

    pub fn union_places(&mut self, left: Place<'tcx>, right: Place<'tcx>) {
        let (Some(left_path), Some(right_path)) =
            (AccessPath::from_place(left), AccessPath::from_place(right))
        else {
            return;
        };
        let left_path = self.normalize_path(&left_path);
        let right_path = self.normalize_path(&right_path);
        self.debug(format_args!("eq {left_path} == {right_path}"));
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
            self.display_places.entry(path.clone()).or_insert(*place);
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

    pub fn set_place_expr(&mut self, place: Place<'tcx>, expr: SymbolicExpr<'tcx>) {
        let Some(path) = AccessPath::from_place(place) else {
            return;
        };
        let path = self.normalize_path(&path);
        self.debug(format_args!("expr {path} ({place:?}) := {expr:?}"));
        self.exprs.insert(path.clone(), expr);
        self.display_places.insert(path, place);
    }

    pub fn expr_for_place(&self, place: Place<'tcx>) -> Option<&SymbolicExpr<'tcx>> {
        let path = self.normalize_path(&AccessPath::from_place(place)?);
        if let Some(expr) = self.exprs.get(&path) {
            return Some(expr);
        }
        self.exprs
            .iter()
            .find(|(expr_path, _)| self.eq.equiv_readonly(path.clone(), (*expr_path).clone()))
            .map(|(_, expr)| expr)
    }

    fn push_fact(&mut self, fact: SymbolicFact<'tcx>) {
        if !self.facts.contains(&fact) {
            self.debug(format_args!("fact {fact:?}"));
            self.facts.push(fact);
        }
    }

    pub fn join(left: &Self, right: &Self) -> Self {
        let mut out = Self {
            eq: join_eq(&left.eq, &right.eq),
            display_places: HashMap::new(),
            exprs: left
                .exprs
                .iter()
                .filter_map(|(path, expr)| {
                    if right.exprs.get(path) == Some(expr) {
                        Some((path.clone(), expr.clone()))
                    } else {
                        None
                    }
                })
                .collect(),
            facts: left
                .facts
                .iter()
                .filter(|fact| right.facts.contains(fact))
                .cloned()
                .collect(),
            points_to: left
                .points_to
                .iter()
                .filter_map(|(path, pointee)| {
                    if right.points_to.get(path) == Some(pointee) {
                        Some((path.clone(), pointee.clone()))
                    } else {
                        None
                    }
                })
                .collect(),
            debug: left.debug || right.debug,
        };
        out.merge_display_places_from(left);
        out.merge_display_places_from(right);
        out
    }
}

fn operand_mentions_path_tree<'tcx>(operand: &Operand<'tcx>, path: &AccessPath) -> bool {
    let (Operand::Copy(place) | Operand::Move(place)) = operand else {
        return false;
    };
    AccessPath::from_place(*place).is_some_and(|operand_path| {
        operand_path.strip_pattern_prefix(path).is_some()
            || path.strip_pattern_prefix(&operand_path).is_some()
    })
}

fn expr_mentions_path_tree<'tcx>(expr: &SymbolicExpr<'tcx>, path: &AccessPath) -> bool {
    match expr {
        SymbolicExpr::Cmp { left, right, .. } => {
            operand_mentions_path_tree(left, path) || operand_mentions_path_tree(right, path)
        }
        SymbolicExpr::Call { args, .. } => args
            .iter()
            .any(|operand| operand_mentions_path_tree(operand, path)),
    }
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
    _tcx: TyCtxt<'tcx>,
    symbolic: &mut SymbolicState<'tcx>,
    stmt: &Statement<'tcx>,
    _local_decls: &LocalDecls<'tcx>,
) {
    let StatementKind::Assign(assign) = &stmt.kind else {
        return;
    };
    let (dst, rvalue) = &**assign;
    if let Some(dst_path) = AccessPath::from_place(*dst) {
        let normalized = symbolic.normalize_path(&dst_path);
        symbolic.kill_path_tree(&normalized);
        if normalized != dst_path {
            symbolic.exprs.remove(&dst_path);
        }
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
        Rvalue::Ref(_, _, borrowed_place) => {
            if let (Some(dst_path), Some(src_path)) = (
                AccessPath::from_place(*dst),
                AccessPath::from_place(*borrowed_place),
            ) {
                let dst_path = symbolic.normalize_path(&dst_path);
                symbolic.set_points_to(dst_path, symbolic.normalize_path(&src_path));
            }
        }
        Rvalue::RawPtr(_, borrowed_place) => {
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

pub fn transfer_terminator<'tcx>(
    tcx: TyCtxt<'tcx>,
    symbolic: &mut SymbolicState<'tcx>,
    term: &Terminator<'tcx>,
    local_decls: &LocalDecls<'tcx>,
) {
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

    let TyKind::FnDef(def_id, _) = func.ty(local_decls, tcx).kind() else {
        return;
    };
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
