use super::reduction::{
    allocation_interval, allocation_nullptr, equivalent_values, symbolic_allocation,
    symbolic_interval, symbolic_nullptr, value_relations,
};
use super::state::CombinedState;
use mirsa_framework::reduction::MAX_RELATED;
use mirsa_relations::symbolic::SymbolicFact;
use rustc_middle::mir::{LocalDecls, Operand, Place, Terminator};
use rustc_middle::ty::{TyCtxt, TyKind};

impl<'tcx> CombinedState<'tcx> {
    pub(super) fn reduce_call_result(
        &mut self,
        tcx: TyCtxt<'tcx>,
        local_decls: &LocalDecls<'tcx>,
        term: &Terminator<'tcx>,
    ) {
        allocation_interval::reduce_call_result(
            tcx,
            local_decls,
            &mut self.interval,
            &self.allocation,
            &self.symbolic,
            term,
        );
    }

    pub(super) fn reduce_local(
        &mut self,
        tcx: TyCtxt<'tcx>,
        local_decls: &LocalDecls<'tcx>,
        place: Place<'tcx>,
    ) -> bool {
        if !equivalent_values::reduce(tcx, local_decls, self, place) {
            return false;
        }
        if mirsa_domains::nullptr::transfer::is_tracked(place.ty(local_decls, tcx).ty) {
            if !allocation_nullptr::reduce(
                &mut self.allocation,
                &mut self.nullptr,
                &self.symbolic,
                place,
            ) {
                return false;
            }
        }
        for fact in self.symbolic.facts_for_place(place, MAX_RELATED) {
            if !self.reduce_fact(tcx, local_decls, &fact) {
                return false;
            }
        }
        if !value_relations::reduce(tcx, local_decls, self, place) {
            return false;
        }
        if matches!(place.ty(local_decls, tcx).ty.kind(), TyKind::Bool) {
            if let Some(value) = self
                .interval
                .tracked_interval_resolved(&self.symbolic, &place)
            {
                if value.low == value.high && matches!(value.low, 0 | 1) {
                    let fact = SymbolicFact::EqConst {
                        expr: Operand::Copy(place),
                        value: value.low as u128,
                    };
                    if !self.reduce_fact(tcx, local_decls, &fact) {
                        return false;
                    }
                }
            }
        }
        true
    }

    fn reduce_fact(
        &mut self,
        tcx: TyCtxt<'tcx>,
        local_decls: &LocalDecls<'tcx>,
        fact: &SymbolicFact<'tcx>,
    ) -> bool {
        symbolic_interval::reduce_fact(
            tcx,
            local_decls,
            &mut self.interval,
            &mut self.symbolic,
            fact,
        ) && symbolic_nullptr::reduce_fact(
            tcx,
            local_decls,
            &mut self.nullptr,
            &mut self.symbolic,
            fact,
        ) && symbolic_allocation::reduce_fact(
            tcx,
            local_decls,
            &mut self.allocation,
            &self.symbolic,
            fact,
        )
    }
}
