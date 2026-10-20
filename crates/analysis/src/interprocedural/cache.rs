use super::context::InputContext;
use super::summary::FunctionSummary;
use rustc_middle::ty::Instance;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

const MAX_VARIANTS: usize = 8;
const MAX_ACTIVE_CALLS: usize = 8;
const MAX_SUMMARIES: usize = 256;

#[derive(Default)]
pub(super) struct SummaryCache<'tcx> {
    summaries: HashMap<Instance<'tcx>, Vec<Rc<FunctionSummary<'tcx>>>>,
    active: HashSet<Instance<'tcx>>,
    count: usize,
    pub hits: usize,
    pub builds: usize,
}

impl<'tcx> SummaryCache<'tcx> {
    pub fn get(
        &mut self,
        instance: Instance<'tcx>,
        input: &InputContext<'tcx>,
    ) -> Option<Rc<FunctionSummary<'tcx>>> {
        let summary = self
            .summaries
            .get(&instance)?
            .iter()
            .find(|summary| &summary.input == input)?
            .clone();
        self.hits += 1;
        Some(summary)
    }

    pub fn begin(&mut self, instance: Instance<'tcx>) -> bool {
        if self.active.len() >= MAX_ACTIVE_CALLS
            || self.count >= MAX_SUMMARIES
            || self
                .summaries
                .get(&instance)
                .is_some_and(|summaries| summaries.len() >= MAX_VARIANTS)
        {
            return false;
        }
        self.active.insert(instance)
    }

    pub fn finish(&mut self, instance: Instance<'tcx>, summary: Rc<FunctionSummary<'tcx>>) {
        self.active.remove(&instance);
        self.count += 1;
        self.builds += 1;
        self.summaries.entry(instance).or_default().push(summary);
    }
}
