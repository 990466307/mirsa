//! Candidate predicates are proved on incoming states before they can survive a join.
use super::{bind::Binder, *};
use crate::reduced_product::CombinedState;
use mirsa_relations::symbolic::PredicateFact;
use rustc_hir::def_id::DefId;
use rustc_middle::mir::BasicBlock;
use serde::Serialize;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;

#[derive(Clone, Debug)]
pub struct Candidate<'tcx> {
    body: DefId,
    block: BasicBlock,
    fact: PredicateFact<'tcx>,
    predicate: Predicate<'tcx>,
}

impl<'tcx> Candidate<'tcx> {
    pub fn new(
        tcx: TyCtxt<'tcx>,
        body: &Body<'tcx>,
        block: BasicBlock,
        expression: &str,
        bindings: &Bindings<'tcx>,
    ) -> Result<Self, Vec<Diagnostic>> {
        let error = |message: &str| {
            vec![Diagnostic {
                span: 0..expression.len(),
                message: message.into(),
            }]
        };
        if block.index() >= body.basic_blocks.len() || body.basic_blocks[block].is_cleanup {
            return Err(error("invalid guidance block"));
        }
        // Object lifetime and memory reads need separate dependency tracking.
        // This first feedback path admits only values held in local variables.
        for value in bindings.values.values() {
            if let Some(place) = value.place() {
                if place.local.index() >= body.local_decls.len() || !place.projection.is_empty() {
                    return Err(error("guidance bindings must be direct locals"));
                }
            }
        }
        let fact = PredicateFact {
            expression: expression.into(),
            values: bindings
                .values
                .iter()
                .map(|(name, value)| (name.clone(), value.clone()))
                .collect(),
        };
        let predicate = bind_fact(tcx, body, &fact)?;
        let mut pure = true;
        predicate.visit_exprs(&mut |expr| {
            if let Expr::TypedExpr(value) = expr {
                match &value.kind {
                    value::TypedKind::Project { .. } => pure = false,
                    value::TypedKind::Builtin(op, _) => {
                        pure &= matches!(
                            op,
                            mirsa_dsl::Builtin::Math
                                | mirsa_dsl::Builtin::Address
                                | mirsa_dsl::Builtin::IsNull
                                | mirsa_dsl::Builtin::Len
                                | mirsa_dsl::Builtin::IsNan
                                | mirsa_dsl::Builtin::IsInfinite
                        )
                    }
                    _ => {}
                }
            }
        });
        if !pure {
            return Err(error(
                "guidance currently supports pure value predicates only",
            ));
        }
        Ok(Self {
            body: body.source.def_id(),
            block,
            fact,
            predicate,
        })
    }
}

pub(super) fn bind_fact<'tcx>(
    tcx: TyCtxt<'tcx>,
    body: &Body<'tcx>,
    fact: &PredicateFact<'tcx>,
) -> Result<Predicate<'tcx>, Vec<Diagnostic>> {
    let syntax = parse_expression(&fact.expression)?;
    let bindings = fact
        .values
        .iter()
        .fold(Bindings::new(), |b, (name, value)| {
            b.value(name, value.clone())
        });
    let binder = Binder {
        tcx,
        body,
        bindings: &bindings,
    };
    let expr = binder
        .bind(&syntax, Some(value::ValueType::Rust(tcx.types.bool)))
        .map_err(|e| vec![e])?;
    Ok(Predicate::Value(expr))
}

#[derive(Clone, Debug, Serialize)]
pub struct GuidanceCheck {
    pub function: String,
    pub predecessor: Option<usize>,
    pub block: usize,
    pub expression: String,
    pub bindings: std::collections::BTreeMap<String, String>,
    pub accepted: bool,
    pub proof: Proof,
    pub elapsed_ms: u128,
}

pub(crate) struct Guidance<'tcx> {
    candidates: HashMap<(DefId, BasicBlock), Vec<Candidate<'tcx>>>,
    config: QueryConfig,
    checks: RefCell<Vec<GuidanceCheck>>,
    omitted: Cell<u64>,
}

impl<'tcx> Guidance<'tcx> {
    pub(crate) fn new(candidates: Vec<Candidate<'tcx>>, config: QueryConfig) -> Self {
        let mut grouped = HashMap::<_, Vec<_>>::new();
        for candidate in candidates {
            grouped
                .entry((candidate.body, candidate.block))
                .or_default()
                .push(candidate);
        }
        Self {
            candidates: grouped,
            config,
            checks: RefCell::new(Vec::new()),
            omitted: Cell::new(0),
        }
    }

    pub(crate) fn checks(&self) -> (Vec<GuidanceCheck>, u64) {
        (self.checks.borrow().clone(), self.omitted.get())
    }

    pub(crate) fn apply(
        &self,
        tcx: TyCtxt<'tcx>,
        body: &Body<'tcx>,
        predecessor: Option<BasicBlock>,
        block: BasicBlock,
        state: &mut CombinedState<'tcx>,
        remaining: Duration,
    ) {
        if !state.feasible {
            return;
        }
        let Some(candidates) = self.candidates.get(&(body.source.def_id(), block)) else {
            return;
        };
        let started = Instant::now();
        for candidate in candidates {
            let config = QueryConfig {
                total_timeout: self
                    .config
                    .total_timeout
                    .min(remaining.saturating_sub(started.elapsed())),
                ..self.config
            };
            let mut view = QueryView {
                tcx,
                body,
                states: vec![(0, true, state.clone())],
                config,
            };
            let before = Instant::now();
            let proof = view.prove(&candidate.predicate);
            let accepted = proof.verdict == Verdict::T && !proof.unreachable;
            // The fact is attached only to the state on which it was proved.
            // Ordinary intersection at joins therefore requires all incoming states.
            if accepted {
                state.symbolic.add_predicate(candidate.fact.clone());
            }
            let mut checks = self.checks.borrow_mut();
            if checks.len() < 4096 {
                checks.push(GuidanceCheck {
                    function: tcx.def_path_str(body.source.def_id()),
                    predecessor: predecessor.map(|p| p.index()),
                    block: block.index(),
                    expression: candidate.fact.expression.clone(),
                    bindings: candidate
                        .fact
                        .values
                        .iter()
                        .map(|(k, v)| (k.clone(), format!("{v:?}")))
                        .collect(),
                    accepted,
                    proof,
                    elapsed_ms: before.elapsed().as_millis(),
                });
            } else {
                self.omitted.set(self.omitted.get() + 1);
            }
        }
    }
}
