use super::{SymbolicExpr, SymbolicFact, SymbolicState, has_runtime_index};
use mirsa_framework::reduction::{MAX_RELATED, MAX_VALUES};
use rustc_middle::{
    mir::{BinOp, LocalDecls, Operand, Place},
    ty::{TyCtxt, TyKind},
};
use std::collections::HashSet;

// Possible outcomes of a total-order comparison: less, equal, greater.
const ALL: u8 = 0b111;
const EQUAL: u8 = 0b010;

fn outcomes(op: BinOp) -> Option<u8> {
    Some(match op {
        BinOp::Lt => 0b001,
        BinOp::Eq => EQUAL,
        BinOp::Gt => 0b100,
        BinOp::Le => 0b011,
        BinOp::Ne => 0b101,
        BinOp::Ge => 0b110,
        _ => return None,
    })
}

impl<'tcx> SymbolicState<'tcx> {
    /// Decide a scalar comparison from equalities and bounded, indexed path facts.
    /// Floating-point comparisons require separate handling of unordered values.
    pub fn compare(
        &self,
        tcx: TyCtxt<'tcx>,
        decls: &LocalDecls<'tcx>,
        op: BinOp,
        left: &Operand<'tcx>,
        right: &Operand<'tcx>,
    ) -> Option<bool> {
        let wanted = outcomes(op)?;
        let possible = self.comparison_outcomes(tcx, decls, left, right)?;
        if possible == 0 {
            None
        } else if possible & wanted == 0 {
            Some(false)
        } else if possible & !wanted == 0 {
            Some(true)
        } else {
            None
        }
    }

    pub fn comparison_may_hold(
        &self,
        tcx: TyCtxt<'tcx>,
        decls: &LocalDecls<'tcx>,
        op: BinOp,
        truth: bool,
        left: &Operand<'tcx>,
        right: &Operand<'tcx>,
    ) -> bool {
        let Some(wanted) = outcomes(op) else {
            return true;
        };
        self.comparison_outcomes(tcx, decls, left, right)
            .is_none_or(|possible| possible & if truth { wanted } else { ALL ^ wanted } != 0)
    }

    fn same_comparison_operand(&self, left: &Operand<'tcx>, right: &Operand<'tcx>) -> bool {
        match (left.place(), right.place()) {
            (Some(a), Some(b)) => {
                !has_runtime_index(a) && !has_runtime_index(b) && self.equiv_places_readonly(a, b)
            }
            (None, None) => left == right,
            _ => false,
        }
    }

    fn comparison_outcomes(
        &self,
        tcx: TyCtxt<'tcx>,
        decls: &LocalDecls<'tcx>,
        left: &Operand<'tcx>,
        right: &Operand<'tcx>,
    ) -> Option<u8> {
        let ty = left.ty(decls, tcx);
        if ty != right.ty(decls, tcx)
            || !matches!(
                ty.kind(),
                TyKind::Int(_) | TyKind::Uint(_) | TyKind::Bool | TyKind::Char
            )
        {
            return None;
        }
        if self.same_comparison_operand(left, right) {
            return Some(EQUAL);
        }
        let matches =
            |a: &Operand<'tcx>, b| a.ty(decls, tcx) == ty && self.same_comparison_operand(a, b);
        let mut possible = ALL;
        for condition in self.comparison_conditions(left, right) {
            if !condition.ty(decls, tcx).ty.is_bool() {
                continue;
            }
            let Some(SymbolicExpr::Cmp {
                op,
                left: a,
                right: b,
            }) = self.expr_for_place(condition)
            else {
                continue;
            };
            let Some(mut allowed) = outcomes(*op) else {
                continue;
            };
            if matches(a, right) && matches(b, left) {
                allowed = ((allowed & 0b001) << 2) | (allowed & EQUAL) | ((allowed & 0b100) >> 2);
            } else if !(matches(a, left) && matches(b, right)) {
                continue;
            }
            for alias in self.equivalent_places(condition, MAX_RELATED) {
                if !alias.ty(decls, tcx).ty.is_bool() {
                    continue;
                }
                for fact in self.facts_for_place(alias, MAX_RELATED) {
                    let truth = match fact {
                        SymbolicFact::EqConst { value: 0, .. }
                        | SymbolicFact::NeConst { value: 1, .. } => false,
                        SymbolicFact::EqConst { value: 1, .. }
                        | SymbolicFact::NeConst { value: 0, .. } => true,
                        _ => continue,
                    };
                    possible &= if truth { allowed } else { ALL ^ allowed };
                }
            }
        }
        Some(possible)
    }

    fn comparison_conditions(
        &self,
        left: &Operand<'tcx>,
        right: &Operand<'tcx>,
    ) -> Vec<Place<'tcx>> {
        let mut seen = HashSet::new();
        let mut out = Vec::new();
        let mut remaining = MAX_VALUES;
        for input in [left, right].into_iter().filter_map(Operand::place) {
            let Some(path) = self.normalize_place(input) else {
                continue;
            };
            for alias in self.eq.members(path).take(MAX_RELATED) {
                for output in self
                    .dependents
                    .get(&alias)
                    .into_iter()
                    .flatten()
                    .take(MAX_RELATED)
                {
                    if remaining == 0 {
                        return out;
                    }
                    remaining -= 1;
                    if seen.insert(output.clone()) {
                        if let Some(place) = self.place_for_path(output) {
                            out.push(place);
                        }
                    }
                }
            }
        }
        out
    }
}
