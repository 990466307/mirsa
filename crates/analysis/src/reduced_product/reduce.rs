use crate::reduced_product::state::CombinedState;
use mirsa_domains::interval::Interval;
use mirsa_domains::interval::abstract_value::{intersect, join as join_interval};
use mirsa_domains::interval::transfer::{
    eval_assign_rhs_interval, eval_operand as eval_interval_operand, switch_value_to_i128,
};
use mirsa_domains::nullptr::NullPtr;
use mirsa_domains::nullptr::abstract_value::join as join_nullptr;
use mirsa_domains::nullptr::transfer::{
    const_nullness, eval_operand as eval_nullptr_operand, get_tracked_value, is_tracked,
};
use mirsa_framework::access_path::AccessPath;
use mirsa_framework::printer::StateEntries;
use mirsa_relations::symbolic::{SymbolicEffect, SymbolicExpr, SymbolicFact};
use rustc_middle::mir::*;
use rustc_middle::ty::{Ty, TyCtxt, TyKind};

impl<'tcx> CombinedState<'tcx> {
    pub fn synchronize_domains(&mut self) {
        self.sync_nullness_from_interval();
        self.interval.merge_display_places_into(&mut self.symbolic);
        self.nullptr.merge_display_places_into(&mut self.symbolic);
    }

    pub fn refine_with_path_facts(
        &mut self,
        tcx: TyCtxt<'tcx>,
        local_decls: &LocalDecls<'tcx>,
    ) -> bool {
        self.synchronize_domains();
        self.consume_symbolic_effects(tcx, local_decls);
        let facts = self.symbolic.facts().to_vec();
        if !facts.is_empty() {
            self.debug_reduce(format_args!("apply {} path facts", facts.len()));
        }
        for fact in facts {
            self.debug_reduce(format_args!("fact {fact:?}"));
            let ok = match fact {
                SymbolicFact::EqConst { expr, value } => {
                    self.refine_eq_const(tcx, local_decls, &expr, value)
                }
                SymbolicFact::NeConst { expr, value } => {
                    self.refine_ne_const(tcx, local_decls, &expr, value)
                }
            };
            if !ok {
                self.debug_reduce(format_args!("contradiction"));
                return false;
            }
        }
        self.synchronize_domains();
        true
    }

    fn consume_symbolic_effects(&mut self, tcx: TyCtxt<'tcx>, local_decls: &LocalDecls<'tcx>) {
        for effect in self.symbolic.take_effects() {
            match effect {
                SymbolicEffect::IndexedRead { dst, src } => {
                    self.materialize_indexed_read(tcx, local_decls, dst, src);
                }
                SymbolicEffect::IndexedWrite { place, rvalue } => {
                    self.materialize_indexed_write(tcx, local_decls, place, &rvalue);
                }
            }
        }
    }

    fn materialize_indexed_read(
        &mut self,
        tcx: TyCtxt<'tcx>,
        local_decls: &LocalDecls<'tcx>,
        dst: Place<'tcx>,
        src: Place<'tcx>,
    ) {
        let dst_ty = dst.ty(local_decls, tcx).ty;
        if is_interval_scalar(dst_ty) {
            let value = self.read_indexed_interval(tcx, local_decls, src);
            self.debug_reduce(format_args!("indexed read {:?} := {}", dst, value));
            self.interval
                .set_interval_resolved(&self.symbolic, dst, value);
        }
        if is_tracked(dst_ty) {
            let value = self.read_indexed_nullptr(src);
            self.debug_reduce(format_args!("indexed read {:?} := {}", dst, value));
            self.nullptr
                .set_place_path_resolved(&self.symbolic, dst, value);
        }
    }

    fn materialize_indexed_write(
        &mut self,
        tcx: TyCtxt<'tcx>,
        local_decls: &LocalDecls<'tcx>,
        place: Place<'tcx>,
        rvalue: &Rvalue<'tcx>,
    ) {
        if let Some(resolved) = self.resolve_indexed_place(tcx, local_decls, place) {
            self.materialize_exact_write(tcx, local_decls, resolved, rvalue);
        } else {
            self.materialize_summary_write(tcx, local_decls, place, rvalue);
        }
    }

    fn materialize_exact_write(
        &mut self,
        tcx: TyCtxt<'tcx>,
        local_decls: &LocalDecls<'tcx>,
        place: Place<'tcx>,
        rvalue: &Rvalue<'tcx>,
    ) {
        let ty = place.ty(local_decls, tcx).ty;
        if is_interval_scalar(ty) {
            let value = eval_assign_rhs_interval(
                tcx,
                &mut self.interval,
                &self.symbolic,
                local_decls,
                rvalue,
            );
            self.debug_reduce(format_args!("indexed write {:?} := {}", place, value));
            self.interval
                .set_interval_resolved(&self.symbolic, place, value);
        }
        if is_tracked(ty) {
            let value = self.eval_nullptr_rvalue(tcx, local_decls, ty, rvalue);
            self.debug_reduce(format_args!("indexed write {:?} := {}", place, value));
            self.nullptr
                .set_place_path_resolved(&self.symbolic, place, value);
        }
    }

    fn materialize_summary_write(
        &mut self,
        tcx: TyCtxt<'tcx>,
        local_decls: &LocalDecls<'tcx>,
        place: Place<'tcx>,
        rvalue: &Rvalue<'tcx>,
    ) {
        let ty = place.ty(local_decls, tcx).ty;
        if is_interval_scalar(ty) {
            let value = eval_assign_rhs_interval(
                tcx,
                &mut self.interval,
                &self.symbolic,
                local_decls,
                rvalue,
            );
            self.debug_reduce(format_args!(
                "indexed summary write {:?} := {}",
                place, value
            ));
            self.interval
                .join_interval_resolved(&self.symbolic, place, value);
        }
        if is_tracked(ty) {
            let Some(path) = self.symbolic.normalize_place(place) else {
                return;
            };
            let value = self.eval_nullptr_rvalue(tcx, local_decls, ty, rvalue);
            self.debug_reduce(format_args!("indexed summary write {path} := {}", value));
            self.nullptr.join_path(path, value);
        }
    }

    fn resolve_indexed_place(
        &mut self,
        tcx: TyCtxt<'tcx>,
        local_decls: &LocalDecls<'tcx>,
        place: Place<'tcx>,
    ) -> Option<Place<'tcx>> {
        let mut resolved = Place::from(place.local);
        for elem in place.projection.iter() {
            match elem {
                ProjectionElem::Index(local) => {
                    let idx_iv = self
                        .interval
                        .read_interval_resolved(&self.symbolic, Place::from(local));
                    if idx_iv.is_empty() || idx_iv.low != idx_iv.high || idx_iv.low < 0 {
                        return None;
                    }
                    let len = match resolved.ty(local_decls, tcx).ty.kind() {
                        TyKind::Array(_, len) => len.try_to_target_usize(tcx)? as u64,
                        _ => return None,
                    };
                    let idx = idx_iv.low as u64;
                    if idx >= len {
                        return None;
                    }
                    resolved = resolved.project_deeper(
                        &[ProjectionElem::ConstantIndex {
                            offset: idx,
                            min_length: len,
                            from_end: false,
                        }],
                        tcx,
                    );
                }
                _ => resolved = resolved.project_deeper(&[elem], tcx),
            }
        }
        Some(resolved)
    }

    fn read_indexed_interval(
        &mut self,
        tcx: TyCtxt<'tcx>,
        local_decls: &LocalDecls<'tcx>,
        src: Place<'tcx>,
    ) -> Interval {
        if let Some(resolved) = self.resolve_indexed_place(tcx, local_decls, src) {
            return self
                .interval
                .read_interval_resolved(&self.symbolic, resolved);
        }

        let mut value = Interval::empty();
        let mut found = false;
        for place in self.projected_interval_places_with_runtime_index(src) {
            let current = self.interval.read_interval_resolved(&self.symbolic, place);
            value = join_interval(&value, &current);
            found = true;
        }
        if let Some(summary) = self
            .interval
            .tracked_interval_resolved(&self.symbolic, &src)
        {
            value = join_interval(&value, &summary);
            found = true;
        }
        if found { value } else { Interval::top() }
    }

    fn read_indexed_nullptr(&self, src: Place<'tcx>) -> NullPtr {
        let Some(pattern) = self.symbolic.normalize_place(src) else {
            return NullPtr::MaybeNull;
        };
        let mut value = NullPtr::Bot;
        let mut found = false;
        for path in self.nullptr.fact_paths() {
            if path.matches_pattern(&pattern) || path == pattern {
                value = join_nullptr(value, self.nullptr.value_or_maybe(&path));
                found = true;
            }
        }
        if found { value } else { NullPtr::MaybeNull }
    }

    fn projected_interval_places_with_runtime_index(&self, place: Place<'tcx>) -> Vec<Place<'tcx>> {
        self.interval
            .interval_places()
            .into_iter()
            .filter(|candidate| runtime_index_place_matches(place, *candidate))
            .collect()
    }

    fn eval_nullptr_rvalue(
        &mut self,
        tcx: TyCtxt<'tcx>,
        local_decls: &LocalDecls<'tcx>,
        dst_ty: Ty<'tcx>,
        rvalue: &Rvalue<'tcx>,
    ) -> NullPtr {
        match rvalue {
            Rvalue::Use(op) | Rvalue::Cast(_, op, _) => eval_nullptr_operand(
                tcx,
                local_decls,
                op,
                &mut self.nullptr,
                &self.symbolic,
                dst_ty,
            ),
            Rvalue::Ref(..) => NullPtr::NonNull,
            Rvalue::RawPtr(..) => NullPtr::MaybeNull,
            _ if matches!(dst_ty.kind(), TyKind::Ref(_, _, _)) => NullPtr::NonNull,
            _ => NullPtr::MaybeNull,
        }
    }

    fn sync_nullness_from_interval(&mut self) {
        let tracked_places: Vec<_> = self
            .nullptr
            .entries()
            .into_iter()
            .map(|(place, _)| place)
            .collect();

        for place in tracked_places {
            let Some(path) = self
                .nullptr
                .access_path_for_place_resolved(&self.symbolic, place)
            else {
                continue;
            };
            if self.nullptr.value_or_maybe(&path) != NullPtr::MaybeNull {
                continue;
            }

            let Some(interval) = self
                .interval
                .tracked_interval_resolved(&self.symbolic, &place)
            else {
                continue;
            };
            if interval.is_empty() {
                continue;
            }

            if interval.low == 0 && interval.high == 0 {
                self.debug_reduce(format_args!(
                    "nullness from interval {path}: {interval} -> null"
                ));
                self.nullptr.set_path(path, NullPtr::Null);
            } else if interval.high < 0 || interval.low > 0 {
                self.debug_reduce(format_args!(
                    "nullness from interval {path}: {interval} -> non-null"
                ));
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
        self.debug_reduce(format_args!("eq const {expr:?} == {value}"));
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
        self.debug_reduce(format_args!("ne const {expr:?} != {value}"));
        if let Some(truth) = self.ne_const_truth(local_decls, tcx, expr, value) {
            if !self.refine_bool_expr(tcx, local_decls, expr, truth) {
                return false;
            }
        }

        let (Operand::Copy(place) | Operand::Move(place)) = expr else {
            return true;
        };
        let current = self.interval.read_interval_resolved(&self.symbolic, *place);
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
            self.debug_reduce(format_args!("no symbolic expr for {place:?}"));
            return true;
        };
        self.debug_reduce(format_args!(
            "bool expr {place:?} is {expr:?}, truth={truth}"
        ));
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
        if !interval_operand(local_decls, tcx, left) || !interval_operand(local_decls, tcx, right) {
            return true;
        }

        let left_iv =
            eval_interval_operand(tcx, local_decls, left, &self.symbolic, &mut self.interval);
        let right_iv =
            eval_interval_operand(tcx, local_decls, right, &self.symbolic, &mut self.interval);
        self.debug_reduce(format_args!(
            "cmp {:?} truth={} left={} right={}",
            op, truth, left_iv, right_iv
        ));
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
        let current = self.interval.read_interval_resolved(&self.symbolic, place);
        if current.is_empty() {
            return true;
        }
        let refined = intersect(&current, &new_iv);
        self.debug_reduce(format_args!(
            "{:?}: {} ∩ {} = {}",
            place, current, new_iv, refined
        ));
        self.interval.debug(format_args!(
            "reduce {:?}: {} ∩ {} = {}",
            place, current, new_iv, refined
        ));
        if refined.is_empty() {
            return false;
        }
        self.interval
            .set_interval_resolved(&self.symbolic, place, refined);

        let all_places = self.interval.interval_places();
        for other in all_places {
            if other == place {
                continue;
            }
            if self.symbolic.equiv_places_readonly(place, other) {
                let other_current = self.interval.read_interval_resolved(&self.symbolic, other);
                if other_current.is_empty() {
                    continue;
                }
                let other_refined = intersect(&other_current, &refined);
                if other_refined.is_empty() {
                    return false;
                }
                self.interval
                    .set_interval_resolved(&self.symbolic, other, other_refined);
            }
        }
        true
    }

    fn refine_is_empty(&mut self, truth: bool, receiver: &Operand<'tcx>) -> bool {
        let (Operand::Copy(place) | Operand::Move(place)) = receiver else {
            return true;
        };
        let current = self
            .interval
            .read_len_resolved_or_top(&self.symbolic, *place);
        let wanted = if truth {
            Interval::new(0, 0)
        } else {
            Interval::new(1, i128::MAX)
        };
        let refined = intersect(&current, &wanted);
        self.debug_reduce(format_args!(
            "is_empty truth={} {:?}.len: {} ∩ {} = {}",
            truth, place, current, wanted, refined
        ));
        self.interval.debug(format_args!(
            "reduce is_empty truth={} {:?}.len: {} ∩ {} = {}",
            truth, place, current, wanted, refined
        ));
        if refined.is_empty() {
            return false;
        }
        self.interval
            .set_len_resolved(&self.symbolic, *place, refined);
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
                        self.nullptr
                            .access_path_for_place_resolved(&self.symbolic, *pl),
                        self.nullptr
                            .access_path_for_place_resolved(&self.symbolic, *pr),
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
        self.debug_reduce(format_args!(
            "nullptr side {:?} equal={} other={other:?} -> {wanted:?}",
            place, equal
        ));
        self.refine_nullptr_place_to(*place, ty, wanted)
    }

    fn operand_nullness(
        &mut self,
        tcx: TyCtxt<'tcx>,
        local_decls: &LocalDecls<'tcx>,
        op: &Operand<'tcx>,
    ) -> Option<NullPtr> {
        match op {
            Operand::Copy(place) | Operand::Move(place) => {
                let ty = place.ty(local_decls, tcx).ty;
                if is_tracked(ty) {
                    Some(get_tracked_value(
                        &mut self.nullptr,
                        &self.symbolic,
                        *place,
                        ty,
                    ))
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
        self.debug_reduce(format_args!(
            "is_null truth={truth} {place:?} -> {wanted:?}"
        ));
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
        let current = get_tracked_value(&mut self.nullptr, &self.symbolic, place, ty);
        if current == NullPtr::Bot {
            return true;
        }
        let Some(refined) = meet_nullptr(current, wanted) else {
            return false;
        };
        self.debug_reduce(format_args!(
            "nullptr {:?}: {:?} ∩ {:?} = {:?}",
            place, current, wanted, refined
        ));
        let Some(path) = self
            .nullptr
            .access_path_for_place_resolved(&self.symbolic, place)
        else {
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
            let other_current = self.nullptr.value_or_maybe(&other);
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

fn interval_operand<'tcx>(
    local_decls: &LocalDecls<'tcx>,
    tcx: TyCtxt<'tcx>,
    operand: &Operand<'tcx>,
) -> bool {
    is_interval_scalar(operand.ty(local_decls, tcx))
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

fn runtime_index_place_matches<'tcx>(pattern: Place<'tcx>, candidate: Place<'tcx>) -> bool {
    if pattern.local != candidate.local || pattern.projection.len() != candidate.projection.len() {
        return false;
    }
    pattern
        .projection
        .iter()
        .zip(candidate.projection.iter())
        .all(|(left, right)| match left {
            ProjectionElem::Index(_) => matches!(right, ProjectionElem::ConstantIndex { .. }),
            _ => left == right,
        })
}

fn meet_nullptr(current: NullPtr, wanted: NullPtr) -> Option<NullPtr> {
    match (current, wanted) {
        (NullPtr::Bot, _) | (_, NullPtr::Bot) => None,
        (NullPtr::MaybeNull, x) | (x, NullPtr::MaybeNull) => Some(x),
        (x, y) if x == y => Some(x),
        _ => None,
    }
}
