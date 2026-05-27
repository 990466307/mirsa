use crate::combined::state::CombinedState;
use crate::framework::access_path::AccessPath;
use crate::framework::printer::StateEntries;
use crate::framework::symbolic::{SymbolicExpr, SymbolicFact};
use crate::interval::Interval;
use crate::interval::abstract_value::intersect;
use crate::interval::transfer::{eval_operand as eval_interval_operand, switch_value_to_i128};
use crate::nullptr::NullPtr;
use crate::nullptr::transfer::{const_nullness, get_tracked_value, is_tracked};
use rustc_middle::mir::*;
use rustc_middle::ty::{Ty, TyCtxt, TyKind};

impl<'tcx> CombinedState<'tcx> {
    pub fn reduce(&mut self) {
        self.reduce_nullness_from_interval();
        self.interval.merge_display_places_into(&mut self.symbolic);
        self.nullptr.merge_display_places_into(&mut self.symbolic);
    }

    pub fn reduce_with_context(
        &mut self,
        tcx: TyCtxt<'tcx>,
        local_decls: &LocalDecls<'tcx>,
    ) -> bool {
        self.reduce();
        let facts = self.symbolic.facts().to_vec();
        for fact in facts {
            let ok = match fact {
                SymbolicFact::EqConst { expr, value } => {
                    self.refine_eq_const(tcx, local_decls, &expr, value)
                }
                SymbolicFact::NeConst { expr, value } => {
                    self.refine_ne_const(tcx, local_decls, &expr, value)
                }
            };
            if !ok {
                return false;
            }
        }
        self.reduce();
        true
    }

    fn reduce_nullness_from_interval(&mut self) {
        let tracked_places: Vec<_> = self
            .nullptr
            .entries()
            .into_iter()
            .map(|(place, _)| place)
            .collect();

        for place in tracked_places {
            let Some(path) = self.nullptr.access_path_for_place(place) else {
                continue;
            };
            if self.nullptr.get_path(&path) != NullPtr::MaybeNull {
                continue;
            }

            let interval = self.interval.get_interval(&place);
            if interval.is_empty() {
                continue;
            }

            if interval.low == 0 && interval.high == 0 {
                self.nullptr.set_path(path, NullPtr::Null);
            } else if interval.high < 0 || interval.low > 0 {
                self.nullptr.set_path(path, NullPtr::NonNull);
            }
        }
    }

    fn refine_eq_const(
        &mut self,
        tcx: TyCtxt<'tcx>,
        local_decls: &LocalDecls<'tcx>,
        expr: &Operand<'tcx>,
        value: u128,
    ) -> bool {
        let truth = self.eq_const_truth(local_decls, tcx, expr, value);
        let mut ok = true;
        let expr_ty = expr.ty(local_decls, tcx);
        if let (Some(place), Some(value)) = (
            interval_place_operand(local_decls, tcx, expr),
            switch_value_to_i128(tcx, expr_ty, value),
        ) {
            ok = self.refine_place_with_interval(place, Interval::new(value, value));
        }
        if ok {
            if let Some(truth) = truth {
                ok = self.refine_bool_expr(tcx, local_decls, expr, truth);
            }
        }
        ok
    }

    fn refine_ne_const(
        &mut self,
        tcx: TyCtxt<'tcx>,
        local_decls: &LocalDecls<'tcx>,
        expr: &Operand<'tcx>,
        value: u128,
    ) -> bool {
        if let Some(truth) = self.ne_const_truth(local_decls, tcx, expr, value) {
            if !self.refine_bool_expr(tcx, local_decls, expr, truth) {
                return false;
            }
        }

        let (Operand::Copy(place) | Operand::Move(place)) = expr else {
            return true;
        };
        let current = self.interval.get_interval(place);
        if current.is_empty() || current.low != current.high {
            return true;
        }
        let expr_ty = expr.ty(local_decls, tcx);
        switch_value_to_i128(tcx, expr_ty, value) != Some(current.low)
    }

    fn eq_const_truth(
        &self,
        local_decls: &LocalDecls<'tcx>,
        tcx: TyCtxt<'tcx>,
        expr: &Operand<'tcx>,
        value: u128,
    ) -> Option<bool> {
        if !matches!(expr.ty(local_decls, tcx).kind(), TyKind::Bool) {
            return None;
        }
        match value {
            0 => Some(false),
            1 => Some(true),
            _ => None,
        }
    }

    fn ne_const_truth(
        &self,
        local_decls: &LocalDecls<'tcx>,
        tcx: TyCtxt<'tcx>,
        expr: &Operand<'tcx>,
        value: u128,
    ) -> Option<bool> {
        if !matches!(expr.ty(local_decls, tcx).kind(), TyKind::Bool) {
            return None;
        }
        match value {
            0 => Some(true),
            1 => Some(false),
            _ => None,
        }
    }

    fn refine_bool_expr(
        &mut self,
        tcx: TyCtxt<'tcx>,
        local_decls: &LocalDecls<'tcx>,
        discr: &Operand<'tcx>,
        truth: bool,
    ) -> bool {
        let (Operand::Copy(place) | Operand::Move(place)) = discr else {
            return true;
        };
        let Some(expr) = self.symbolic.expr_for_place(*place).cloned() else {
            return true;
        };
        match expr {
            SymbolicExpr::Cmp { op, left, right } => {
                self.refine_interval_cmp(tcx, local_decls, op, truth, &left, &right)
                    && self.refine_nullptr_cmp(tcx, local_decls, op, truth, &left, &right)
            }
            SymbolicExpr::IsEmpty { receiver } => self.refine_is_empty(truth, &receiver),
            SymbolicExpr::IsNull { arg } => self.refine_is_null(tcx, local_decls, truth, &arg),
        }
    }

    fn refine_interval_cmp(
        &mut self,
        tcx: TyCtxt<'tcx>,
        local_decls: &LocalDecls<'tcx>,
        op: BinOp,
        truth: bool,
        left: &Operand<'tcx>,
        right: &Operand<'tcx>,
    ) -> bool {
        let left_iv = eval_interval_operand(tcx, local_decls, left, &self.interval);
        let right_iv = eval_interval_operand(tcx, local_decls, right, &self.interval);
        self.interval.debug(format_args!(
            "reduce cmp {:?} truth={} left={} right={}",
            op, truth, left_iv, right_iv
        ));

        let left_singleton = if left_iv.low == left_iv.high {
            Some(left_iv.low)
        } else {
            None
        };
        let right_singleton = if right_iv.low == right_iv.high {
            Some(right_iv.low)
        } else {
            None
        };

        match (op, truth) {
            (BinOp::Eq, true) | (BinOp::Ne, false) => {
                let intersected = intersect(&left_iv, &right_iv);
                if intersected.is_empty() && !left_iv.is_empty() && !right_iv.is_empty() {
                    return false;
                }
                if let Some(p) = interval_place_operand(local_decls, tcx, left) {
                    if !self.refine_place_with_interval(p, intersected) {
                        return false;
                    }
                }
                if let Some(p) = interval_place_operand(local_decls, tcx, right) {
                    if !self.refine_place_with_interval(p, intersected) {
                        return false;
                    }
                }
                if let (Some(pl), Some(pr)) = (
                    interval_place_operand(local_decls, tcx, left),
                    interval_place_operand(local_decls, tcx, right),
                ) {
                    self.symbolic.union_places(pl, pr);
                    self.interval.debug(format_args!("eq {:?} == {:?}", pl, pr));
                }
            }
            (BinOp::Eq, false) | (BinOp::Ne, true) => {
                if left_singleton.is_some()
                    && right_singleton.is_some()
                    && left_singleton == right_singleton
                    && !left_iv.is_empty()
                    && !right_iv.is_empty()
                {
                    return false;
                }
            }
            (BinOp::Lt, true) | (BinOp::Ge, false) => {
                if let (Some(c), Some(p)) = (
                    right_singleton,
                    interval_place_operand(local_decls, tcx, left),
                ) {
                    if !self.refine_place_with_interval(
                        p,
                        Interval::new(i128::MIN, c.saturating_sub(1)),
                    ) {
                        return false;
                    }
                }
                if let (Some(c), Some(p)) = (
                    left_singleton,
                    interval_place_operand(local_decls, tcx, right),
                ) {
                    if !self.refine_place_with_interval(
                        p,
                        Interval::new(c.saturating_add(1), i128::MAX),
                    ) {
                        return false;
                    }
                }
            }
            (BinOp::Lt, false) | (BinOp::Ge, true) => {
                if let (Some(c), Some(p)) = (
                    right_singleton,
                    interval_place_operand(local_decls, tcx, left),
                ) {
                    if !self.refine_place_with_interval(p, Interval::new(c, i128::MAX)) {
                        return false;
                    }
                }
                if let (Some(c), Some(p)) = (
                    left_singleton,
                    interval_place_operand(local_decls, tcx, right),
                ) {
                    if !self.refine_place_with_interval(p, Interval::new(i128::MIN, c)) {
                        return false;
                    }
                }
            }
            (BinOp::Le, true) | (BinOp::Gt, false) => {
                if let (Some(c), Some(p)) = (
                    right_singleton,
                    interval_place_operand(local_decls, tcx, left),
                ) {
                    if !self.refine_place_with_interval(p, Interval::new(i128::MIN, c)) {
                        return false;
                    }
                }
                if let (Some(c), Some(p)) = (
                    left_singleton,
                    interval_place_operand(local_decls, tcx, right),
                ) {
                    if !self.refine_place_with_interval(p, Interval::new(c, i128::MAX)) {
                        return false;
                    }
                }
            }
            (BinOp::Le, false) | (BinOp::Gt, true) => {
                if let (Some(c), Some(p)) = (
                    right_singleton,
                    interval_place_operand(local_decls, tcx, left),
                ) {
                    if !self.refine_place_with_interval(
                        p,
                        Interval::new(c.saturating_add(1), i128::MAX),
                    ) {
                        return false;
                    }
                }
                if let (Some(c), Some(p)) = (
                    left_singleton,
                    interval_place_operand(local_decls, tcx, right),
                ) {
                    if !self.refine_place_with_interval(
                        p,
                        Interval::new(i128::MIN, c.saturating_sub(1)),
                    ) {
                        return false;
                    }
                }
            }
            _ => {}
        }

        true
    }

    fn refine_place_with_interval(&mut self, place: Place<'tcx>, new_iv: Interval) -> bool {
        let current = self.interval.get_interval(&place);
        if current.is_empty() {
            return true;
        }
        let refined = intersect(&current, &new_iv);
        self.interval.debug(format_args!(
            "reduce {:?}: {} ∩ {} = {}",
            place, current, new_iv, refined
        ));
        if refined.is_empty() {
            return false;
        }
        self.interval.set_interval(place, refined);

        let all_places = self.interval.interval_places();
        for other in all_places {
            if other == place {
                continue;
            }
            if self.symbolic.equiv_places_readonly(place, other) {
                let other_current = self.interval.get_interval(&other);
                if other_current.is_empty() {
                    continue;
                }
                let other_refined = intersect(&other_current, &refined);
                if other_refined.is_empty() {
                    return false;
                }
                self.interval.set_interval(other, other_refined);
            }
        }
        true
    }

    fn refine_is_empty(&mut self, truth: bool, receiver: &Operand<'tcx>) -> bool {
        let (Operand::Copy(place) | Operand::Move(place)) = receiver else {
            return true;
        };
        let current = self.interval.get_len(place).unwrap_or_else(Interval::top);
        let wanted = if truth {
            Interval::new(0, 0)
        } else {
            Interval::new(1, i128::MAX)
        };
        let refined = intersect(&current, &wanted);
        self.interval.debug(format_args!(
            "reduce is_empty truth={} {:?}.len: {} ∩ {} = {}",
            truth, place, current, wanted, refined
        ));
        if refined.is_empty() {
            return false;
        }
        self.interval.set_len(*place, refined);
        true
    }

    fn refine_nullptr_cmp(
        &mut self,
        tcx: TyCtxt<'tcx>,
        local_decls: &LocalDecls<'tcx>,
        op: BinOp,
        truth: bool,
        left: &Operand<'tcx>,
        right: &Operand<'tcx>,
    ) -> bool {
        let equal = match op {
            BinOp::Eq => truth,
            BinOp::Ne => !truth,
            _ => return true,
        };

        if !self.refine_nullptr_side(tcx, local_decls, equal, left, right) {
            return false;
        }
        if !self.refine_nullptr_side(tcx, local_decls, equal, right, left) {
            return false;
        }

        if equal {
            if let (Operand::Copy(pl) | Operand::Move(pl), Operand::Copy(pr) | Operand::Move(pr)) =
                (left, right)
            {
                let left_ty = pl.ty(local_decls, tcx).ty;
                let right_ty = pr.ty(local_decls, tcx).ty;
                if is_tracked(left_ty) && is_tracked(right_ty) {
                    if let (Some(left_path), Some(right_path)) = (
                        self.nullptr.access_path_for_place(*pl),
                        self.nullptr.access_path_for_place(*pr),
                    ) {
                        self.nullptr
                            .debug(format_args!("eq {left_path} == {right_path}"));
                        self.symbolic.eq.union(left_path, right_path);
                    }
                }
            }
        }

        true
    }

    fn refine_nullptr_side(
        &mut self,
        tcx: TyCtxt<'tcx>,
        local_decls: &LocalDecls<'tcx>,
        equal: bool,
        candidate: &Operand<'tcx>,
        other_op: &Operand<'tcx>,
    ) -> bool {
        let (Operand::Copy(place) | Operand::Move(place)) = candidate else {
            return true;
        };
        let ty = place.ty(local_decls, tcx).ty;
        if !is_tracked(ty) {
            return true;
        }

        let Some(other) = self.operand_nullness(tcx, local_decls, other_op) else {
            return true;
        };
        let wanted = match (equal, other) {
            (true, NullPtr::Null) => Some(NullPtr::Null),
            (true, NullPtr::NonNull) => Some(NullPtr::NonNull),
            (false, NullPtr::Null) => Some(NullPtr::NonNull),
            _ => None,
        };
        let Some(wanted) = wanted else {
            return true;
        };
        self.refine_nullptr_place_to(*place, ty, wanted)
    }

    fn operand_nullness(
        &self,
        tcx: TyCtxt<'tcx>,
        local_decls: &LocalDecls<'tcx>,
        op: &Operand<'tcx>,
    ) -> Option<NullPtr> {
        match op {
            Operand::Copy(place) | Operand::Move(place) => {
                let ty = place.ty(local_decls, tcx).ty;
                if is_tracked(ty) {
                    Some(get_tracked_value(&self.nullptr, *place, ty))
                } else {
                    None
                }
            }
            Operand::Constant(c) => const_nullness(tcx, c),
        }
    }

    fn refine_is_null(
        &mut self,
        tcx: TyCtxt<'tcx>,
        local_decls: &LocalDecls<'tcx>,
        truth: bool,
        arg: &Operand<'tcx>,
    ) -> bool {
        let (Operand::Copy(place) | Operand::Move(place)) = arg else {
            return true;
        };
        let ty = place.ty(local_decls, tcx).ty;
        if !is_tracked(ty) {
            return true;
        }

        let wanted = if truth {
            NullPtr::Null
        } else {
            NullPtr::NonNull
        };
        self.refine_nullptr_place_to(*place, ty, wanted)
    }

    fn refine_nullptr_place_to(
        &mut self,
        place: Place<'tcx>,
        ty: Ty<'tcx>,
        wanted: NullPtr,
    ) -> bool {
        if !is_tracked(ty) {
            return true;
        }
        let current = get_tracked_value(&self.nullptr, place, ty);
        if current == NullPtr::Bot {
            return true;
        }
        let Some(refined) = meet_nullptr(current, wanted) else {
            return false;
        };
        let Some(path) = self.nullptr.access_path_for_place(place) else {
            return true;
        };
        self.refine_tracked_fact(path, refined)
    }

    fn refine_tracked_fact(&mut self, path: AccessPath, refined: NullPtr) -> bool {
        self.nullptr.set_path(path.clone(), refined);
        let all_paths: Vec<AccessPath> = self.nullptr.fact_paths().collect();
        for other in all_paths {
            if other == path || !self.symbolic.eq.equiv_readonly(path.clone(), other.clone()) {
                continue;
            }
            let other_current = self.nullptr.get_path(&other);
            if other_current == NullPtr::Bot {
                continue;
            }
            let Some(other_refined) = meet_nullptr(other_current, refined) else {
                return false;
            };
            self.nullptr.set_path(other, other_refined);
        }
        true
    }
}

fn is_interval_scalar(ty: Ty<'_>) -> bool {
    matches!(
        ty.kind(),
        TyKind::Int(_) | TyKind::Uint(_) | TyKind::Bool | TyKind::Char
    )
}

fn interval_place_operand<'tcx>(
    local_decls: &LocalDecls<'tcx>,
    tcx: TyCtxt<'tcx>,
    operand: &Operand<'tcx>,
) -> Option<Place<'tcx>> {
    let (Operand::Copy(place) | Operand::Move(place)) = operand else {
        return None;
    };
    if is_interval_scalar(place.ty(local_decls, tcx).ty) {
        Some(*place)
    } else {
        None
    }
}

fn meet_nullptr(current: NullPtr, wanted: NullPtr) -> Option<NullPtr> {
    match (current, wanted) {
        (NullPtr::Bot, _) | (_, NullPtr::Bot) => None,
        (NullPtr::MaybeNull, x) | (x, NullPtr::MaybeNull) => Some(x),
        (x, y) if x == y => Some(x),
        _ => None,
    }
}
