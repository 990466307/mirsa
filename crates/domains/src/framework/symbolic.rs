use crate::framework::access_path::AccessPath;
use crate::framework::eq_domain::{EqDomain, join_eq};
use rustc_middle::mir::{
    BinOp, CastKind, LocalDecls, Operand, Place, Rvalue, Statement, StatementKind, Terminator,
    TerminatorKind,
};
use rustc_middle::ty::{TyCtxt, TyKind};
use std::collections::{HashMap, HashSet};

#[derive(Clone, Debug, PartialEq)]
pub enum SymbolicExpr<'tcx> {
    Cmp {
        op: BinOp,
        left: Operand<'tcx>,
        right: Operand<'tcx>,
    },
    IsEmpty {
        receiver: Operand<'tcx>,
    },
    IsNull {
        arg: Operand<'tcx>,
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
}

impl<'tcx> Eq for SymbolicState<'tcx> {}

impl<'tcx> SymbolicState<'tcx> {
    pub fn new() -> Self {
        Self {
            eq: EqDomain::new(),
            display_places: HashMap::new(),
            exprs: HashMap::new(),
            facts: Vec::new(),
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

    pub fn display_place(&self, path: &AccessPath) -> Option<Place<'tcx>> {
        self.display_places.get(path).copied()
    }

    pub fn display_paths(&self) -> impl Iterator<Item = &AccessPath> {
        self.display_places.keys()
    }

    pub fn kill_place(&mut self, place: Place<'tcx>) {
        if let Some(path) = AccessPath::from_place(place) {
            self.eq.kill(path.clone());
            self.exprs.remove(&path);
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
        }
        self.exprs
            .retain(|_, expr| !expr_mentions_path_tree(expr, path));
        self.facts
            .retain(|fact| !fact_mentions_path_tree(fact, path));
    }

    pub fn union_places(&mut self, left: Place<'tcx>, right: Place<'tcx>) {
        let (Some(left_path), Some(right_path)) =
            (AccessPath::from_place(left), AccessPath::from_place(right))
        else {
            return;
        };
        self.eq.union(left_path, right_path);
    }

    pub fn equiv_places_readonly(&self, left: Place<'tcx>, right: Place<'tcx>) -> bool {
        let (Some(left_path), Some(right_path)) =
            (AccessPath::from_place(left), AccessPath::from_place(right))
        else {
            return false;
        };
        self.eq.equiv_readonly(left_path, right_path)
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
        self.exprs.insert(path.clone(), expr);
        self.display_places.insert(path, place);
    }

    pub fn expr_for_place(&self, place: Place<'tcx>) -> Option<&SymbolicExpr<'tcx>> {
        let path = AccessPath::from_place(place)?;
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
        SymbolicExpr::IsEmpty { receiver } => operand_mentions_path_tree(receiver, path),
        SymbolicExpr::IsNull { arg } => operand_mentions_path_tree(arg, path),
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

fn is_slice_is_empty_path(path: &str) -> bool {
    path.ends_with("::is_empty")
}

fn is_ptr_is_null_path(path: &str) -> bool {
    path.ends_with("::is_null") && path.contains("::ptr::")
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
    symbolic.kill_place_tree(*dst);

    match rvalue {
        Rvalue::Use(Operand::Copy(src) | Operand::Move(src)) => {
            let expr = symbolic.expr_for_place(*src).cloned();
            symbolic.union_places(*dst, *src);
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
            let src_ty = src.ty(local_decls, tcx).ty;
            if let TyKind::Ref(_, inner, _) = src_ty.kind() {
                if matches!(inner.kind(), TyKind::Array(_, _)) {
                    symbolic.union_places(*dst, *src);
                }
            }
        }
        Rvalue::Ref(_, _, borrowed_place) => {
            let borrowed_ty = borrowed_place.ty(local_decls, tcx).ty;
            if matches!(borrowed_ty.kind(), TyKind::Array(_, _) | TyKind::Slice(_)) {
                symbolic.union_places(*dst, *borrowed_place);
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
    let path = tcx.def_path_str(*def_id);
    if is_slice_is_empty_path(&path) {
        if let Some(arg) = args.first() {
            symbolic.set_place_expr(
                *destination,
                SymbolicExpr::IsEmpty {
                    receiver: arg.node.clone(),
                },
            );
        }
    } else if is_ptr_is_null_path(&path) {
        if let Some(arg) = args.first() {
            symbolic.set_place_expr(
                *destination,
                SymbolicExpr::IsNull {
                    arg: arg.node.clone(),
                },
            );
        }
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
