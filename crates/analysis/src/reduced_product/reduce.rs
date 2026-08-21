use super::reduction::{
    allocation_interval, allocation_nullptr, interval, interval_nullptr, symbolic_allocation,
    symbolic_interval, symbolic_nullptr,
};
use super::state::CombinedState;
use mirsa_relations::symbolic::SymbolicFact;
use rustc_middle::mir::{LocalDecls, Statement, Terminator};
use rustc_middle::ty::TyCtxt;

impl<'tcx> CombinedState<'tcx> {
    /// Run the reductions that connect component domains. This dispatcher
    /// contains no domain-specific reduction algorithm.
    pub fn synchronize_domains(&mut self) {
        interval_nullptr::reduce(&self.interval, &mut self.nullptr, &self.symbolic);
        allocation_nullptr::reduce(&mut self.allocation, &mut self.nullptr, &self.symbolic);
        self.interval.merge_display_places_into(&mut self.symbolic);
        self.nullptr.merge_display_places_into(&mut self.symbolic);
        self.allocation
            .merge_display_places_into(&mut self.symbolic);
    }

    pub fn reduce_statement(
        &mut self,
        tcx: TyCtxt<'tcx>,
        local_decls: &LocalDecls<'tcx>,
        statement: &Statement<'tcx>,
    ) {
        interval::reduce_statement(
            tcx,
            &mut self.interval,
            &self.symbolic,
            statement,
            local_decls,
        );
        interval_nullptr::reduce_statement(
            tcx,
            &mut self.interval,
            &mut self.nullptr,
            &self.symbolic,
            statement,
            local_decls,
        );
        allocation_interval::reduce_statement(
            tcx,
            &mut self.allocation,
            &mut self.interval,
            &self.symbolic,
            statement,
            local_decls,
        );
        self.synchronize_domains();
    }

    pub fn reduce_terminator(
        &mut self,
        tcx: TyCtxt<'tcx>,
        local_decls: &LocalDecls<'tcx>,
        term: &Terminator<'tcx>,
    ) {
        allocation_interval::reduce_terminator(
            tcx,
            local_decls,
            &mut self.interval,
            &self.allocation,
            &self.symbolic,
            term,
        );
        self.synchronize_domains();
    }

    pub fn refine_with_path_facts(
        &mut self,
        tcx: TyCtxt<'tcx>,
        local_decls: &LocalDecls<'tcx>,
    ) -> bool {
        self.synchronize_domains();
        for fact in self.symbolic.facts().to_vec() {
            self.debug_reduce(format_args!("fact {fact:?}"));
            if !self.reduce_fact(tcx, local_decls, &fact) {
                self.debug_reduce(format_args!("contradiction"));
                return false;
            }
        }
        self.synchronize_domains();
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
