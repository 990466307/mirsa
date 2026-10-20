mod apply;
mod cache;
mod context;
mod summary;

use crate::reduced_product::CombinedState;
use crate::reduced_product::engine::{invalidate_memory, transfer_builtin_terminator};
use cache::SummaryCache;
use context::{CallContext, resolve_body};
use mirsa_framework::forward::PathForwardAnalysisConfig;
use rustc_middle::mir::{BasicBlock, Body, Location, Operand, TerminatorKind};
use rustc_middle::ty::{Instance, TyCtxt};
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::{Duration, Instant};
use summary::ExitEffects;

pub struct AnalysisSession<'tcx> {
    cache: RefCell<SummaryCache<'tcx>>,
    config: PathForwardAnalysisConfig,
    started: Cell<Option<Instant>>,
    guidance: Option<crate::query::guidance::Guidance<'tcx>>,
    slice_stats: RefCell<Option<crate::query::SliceStats>>,
}

impl<'tcx> AnalysisSession<'tcx> {
    pub fn new(config: PathForwardAnalysisConfig) -> Self {
        Self {
            cache: RefCell::new(SummaryCache::default()),
            config,
            started: Cell::new(None),
            guidance: None,
            slice_stats: RefCell::new(None),
        }
    }

    pub fn with_guidance(
        mut self,
        candidates: Vec<crate::query::Candidate<'tcx>>,
        config: crate::query::QueryConfig,
    ) -> Self {
        *self.cache.get_mut() = SummaryCache::default();
        self.guidance = Some(crate::query::guidance::Guidance::new(candidates, config));
        self
    }

    pub fn guidance_checks(&self) -> (Vec<crate::query::GuidanceCheck>, u64) {
        self.guidance
            .as_ref()
            .map(|g| g.checks())
            .unwrap_or_default()
    }

    pub(crate) fn apply_guidance(
        &self,
        tcx: TyCtxt<'tcx>,
        body: &Body<'tcx>,
        predecessor: Option<BasicBlock>,
        block: BasicBlock,
        state: &mut CombinedState<'tcx>,
    ) {
        if let Some(guidance) = &self.guidance {
            let remaining = self
                .remaining_config()
                .max_duration
                .unwrap_or(std::time::Duration::MAX);
            guidance.apply(tcx, body, predecessor, block, state, remaining);
        }
    }

    pub fn cache_stats(&self) -> (usize, usize) {
        let cache = self.cache.borrow();
        (cache.builds, cache.hits)
    }

    pub fn analyze(
        &self,
        tcx: TyCtxt<'tcx>,
        body: &Body<'tcx>,
    ) -> mirsa_framework::forward::PathForwardAnalysisResult<CombinedState<'tcx>> {
        *self.slice_stats.borrow_mut() = None;
        let all_places = mirsa_core::mir::collect_body_places(tcx, body);
        let places = mirsa_core::mir::collect_interval_places(tcx, body);
        let pointers = mirsa_core::mir::collect_ptr_places(tcx, body);
        crate::reduced_product::engine::analyze_with_entry(
            tcx,
            body,
            &mirsa_core::cfg::build_cfg(body),
            &places,
            &all_places,
            &pointers,
            self.remaining_config(),
            Default::default(),
            self,
            None,
            None,
        )
    }

    pub fn analyze_for_queries<'a>(
        &'a self,
        tcx: TyCtxt<'tcx>,
        body: &'a Body<'tcx>,
    ) -> crate::query::AnalysisView<'a, 'tcx> {
        crate::query::AnalysisView::from_session(tcx, body, self)
    }

    pub fn slice_stats(&self) -> Option<crate::query::SliceStats> {
        *self.slice_stats.borrow()
    }

    pub(crate) fn analyze_slice(
        &self,
        tcx: TyCtxt<'tcx>,
        body: &Body<'tcx>,
        slice: &crate::query::slice::CfgSlice,
    ) -> mirsa_framework::forward::PathForwardAnalysisResult<CombinedState<'tcx>> {
        let started = Instant::now();
        // Candidate validation needs its own complete input state. Keep full
        // analysis when guidance is configured; SMT demand slicing still applies.
        if self.guidance.is_some() {
            *self.slice_stats.borrow_mut() = None;
            return self.analyze(tcx, body);
        }
        let all: Vec<_> = mirsa_core::mir::collect_body_places(tcx, body)
            .into_iter()
            .filter(|p| slice.tracks(p))
            .collect();
        let places: Vec<_> = mirsa_core::mir::collect_interval_places(tcx, body)
            .into_iter()
            .filter(|p| slice.tracks(p))
            .collect();
        let pointers: Vec<_> = mirsa_core::mir::collect_ptr_places(tcx, body)
            .into_iter()
            .filter(|p| slice.tracks(p))
            .collect();
        let result = crate::reduced_product::engine::analyze_with_entry(
            tcx,
            body,
            &slice.cfg,
            &places,
            &all,
            &pointers,
            self.remaining_config(),
            Default::default(),
            self,
            None,
            Some(slice),
        );
        *self.slice_stats.borrow_mut() = Some(crate::query::SliceStats {
            analysis_time: started.elapsed(),
            ..slice.stats
        });
        result
    }

    fn remaining_config(&self) -> PathForwardAnalysisConfig {
        let started = self.started.get().unwrap_or_else(|| {
            let now = Instant::now();
            self.started.set(Some(now));
            now
        });
        let limit = self.config.max_duration.unwrap_or(Duration::from_secs(30));
        PathForwardAnalysisConfig {
            max_duration: Some(limit.saturating_sub(started.elapsed())),
            ..self.config
        }
    }

    pub(crate) fn transfer_call(
        &self,
        tcx: TyCtxt<'tcx>,
        body: &Body<'tcx>,
        pred: BasicBlock,
        succ: BasicBlock,
        state: &CombinedState<'tcx>,
    ) -> Option<CombinedState<'tcx>> {
        if !state.feasible {
            return None;
        }
        let term = body.basic_blocks[pred].terminator();
        let TerminatorKind::Call {
            func,
            args,
            destination,
            target,
            ..
        } = &term.kind
        else {
            return None;
        };
        let normal = *target == Some(succ);
        let mut input = state.clone();
        let arguments: Vec<_> = args.iter().map(|arg| arg.node.clone()).collect();
        if let Some((instance, context, summary)) =
            self.summary_for_call(tcx, body, func, &arguments, &input)
        {
            let effects = if normal {
                &summary.normal
            } else {
                &summary.unwind
            };
            match effects {
                ExitEffects::Unreachable => return None,
                ExitEffects::State(_) => {
                    let location = Location {
                        block: pred,
                        statement_index: body.basic_blocks[pred].statements.len(),
                    };
                    if let Some(out) = apply::apply(
                        tcx,
                        body,
                        location,
                        instance.def_id(),
                        &input,
                        &context,
                        *destination,
                        effects,
                    ) {
                        return Some(out);
                    }
                }
                ExitEffects::Unknown => {}
            }
        }
        let unknown_local = matches!(func.ty(&body.local_decls, tcx).kind(), rustc_middle::ty::TyKind::FnDef(id, _) if id.is_local());
        if unknown_local || !normal {
            for arg in &arguments {
                if matches!(
                    arg.ty(&body.local_decls, tcx).kind(),
                    rustc_middle::ty::TyKind::RawPtr(..)
                ) {
                    let pointer = mirsa_domains::allocation::transfer::eval_pointer_operand(
                        tcx,
                        &input.symbolic,
                        &input.allocation,
                        arg,
                        &body.local_decls,
                        arg.ty(&body.local_decls, tcx),
                    );
                    input.allocation.deallocate(&pointer, true);
                }
            }
        }
        if normal {
            transfer_builtin_terminator(tcx, &mut input, term, &body.local_decls);
        } else {
            let written = input
                .symbolic
                .call_write_paths(tcx, term, &body.local_decls);
            invalidate_memory(&mut input, &written);
        }
        input.feasible.then_some(input)
    }
    fn summary_for_call(
        &self,
        tcx: TyCtxt<'tcx>,
        body: &Body<'tcx>,
        func: &Operand<'tcx>,
        args: &[Operand<'tcx>],
        input: &CombinedState<'tcx>,
    ) -> Option<(
        Instance<'tcx>,
        CallContext<'tcx>,
        Rc<summary::FunctionSummary<'tcx>>,
    )> {
        if self
            .remaining_config()
            .max_duration
            .is_some_and(|remaining| remaining.is_zero())
        {
            return None;
        }
        let (instance, callee) = resolve_body(tcx, func, &body.local_decls)?;
        if args.len() != callee.arg_count
            || args.iter().enumerate().any(|(index, arg)| {
                arg.ty(&body.local_decls, tcx)
                    != callee.local_decls[rustc_middle::mir::Local::from_usize(index + 1)].ty
            })
        {
            return None;
        }
        let mut context = CallContext::new(tcx, input, &body.local_decls, &callee, args)?;
        context.input.state.writes = None;
        let cached = self.cache.borrow_mut().get(instance, &context.input);
        let summary = match cached {
            Some(summary) => summary,
            None => {
                if !self.cache.borrow_mut().begin(instance) {
                    return None;
                }
                let summary = Rc::new(summary::build(tcx, self, &callee, context.input.clone()));
                self.cache.borrow_mut().finish(instance, summary.clone());
                summary
            }
        };
        Some((instance, context, summary))
    }
}
