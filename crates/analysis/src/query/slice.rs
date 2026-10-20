//! Conservative backwards CFG slice. Location indices and the original MIR stay intact.
use super::demand::Demand;
use mirsa_core::cfg::{Cfg, build_cfg};
use rustc_middle::mir::visit::{PlaceContext, Visitor};
use rustc_middle::mir::{Body, Local, Location, Place, ProjectionElem, Rvalue, StatementKind};
use std::collections::{HashMap, HashSet, VecDeque};

#[derive(Clone, Copy, Debug, Default)]
pub struct SliceStats {
    pub total_blocks: usize,
    pub retained_blocks: usize,
    pub total_statements: usize,
    pub retained_statements: usize,
    pub analysis_time: std::time::Duration,
}

pub(crate) struct CfgSlice {
    pub cfg: Cfg,
    blocks: Vec<bool>,
    pub statements: Vec<Vec<bool>>,
    pub locals: HashSet<Local>,
    pub stats: SliceStats,
}
#[derive(Default)]
struct Locals(HashSet<Local>);
impl<'tcx> Visitor<'tcx> for Locals {
    fn visit_local(&mut self, local: Local, _: PlaceContext, _: Location) {
        self.0.insert(local);
    }
}
impl CfgSlice {
    pub fn new(body: &Body<'_>, targets: &[Location], demand: Demand) -> Self {
        let mut cfg = build_cfg(body);
        let mut blocks = vec![false; body.basic_blocks.len()];
        let mut pending: Vec<_> = targets.iter().map(|p| p.block).collect();
        while let Some(bb) = pending.pop() {
            if std::mem::replace(&mut blocks[bb.index()], true) {
                continue;
            }
            pending.extend(cfg.pred[bb.index()].iter().copied());
        }
        // Keep the original outgoing edges of retained blocks. Destinations
        // outside the slice are rejected by the analysis before their transfer;
        // the path engine preserves observations across repeated loop visits.
        for (index, edges) in cfg.succ.iter_mut().enumerate() {
            if !blocks[index] {
                edges.clear();
            }
        }
        for edges in &mut cfg.pred {
            edges.retain(|b| blocks[b.index()]);
        }
        // An indirect write or a call may reach any address-taken local. Keep
        // all definitions of these locals, even if alias resolution is unknown.
        let mut address_taken = HashSet::new();
        for data in body.basic_blocks.iter() {
            for statement in &data.statements {
                if let StatementKind::Assign(a) = &statement.kind {
                    if let Rvalue::Ref(_, _, p) | Rvalue::RawPtr(_, p) = &a.1 {
                        address_taken.insert(p.local);
                    }
                }
            }
        }
        let mut locals = demand.locals;
        locals.extend(address_taken);
        let mut statements: Vec<Vec<bool>> = body
            .basic_blocks
            .iter()
            .map(|b| vec![false; b.statements.len()])
            .collect();
        let mut definitions: HashMap<Local, Vec<Location>> = HashMap::new();
        for (bb, data) in body.basic_blocks.iter_enumerated() {
            if !blocks[bb.index()] {
                continue;
            }
            // Preserve every branch/assert and every call on a route to a
            // query. This includes unknown side effects and loop exit guards.
            if cfg.succ[bb.index()].iter().any(|b| blocks[b.index()]) {
                let mut used = Locals::default();
                used.visit_terminator(
                    data.terminator(),
                    Location {
                        block: bb,
                        statement_index: data.statements.len(),
                    },
                );
                locals.extend(used.0);
            }
            for (index, statement) in data.statements.iter().enumerate() {
                let at = Location {
                    block: bb,
                    statement_index: index,
                };
                match &statement.kind {
                    StatementKind::Assign(a)
                        if !a
                            .0
                            .projection
                            .iter()
                            .any(|e| matches!(e, ProjectionElem::Deref)) =>
                    {
                        definitions.entry(a.0.local).or_default().push(at);
                    }
                    StatementKind::StorageLive(local) | StatementKind::StorageDead(local) => {
                        definitions.entry(*local).or_default().push(at);
                    }
                    _ => {
                        statements[bb.index()][index] = true;
                        let mut used = Locals::default();
                        used.visit_statement(statement, at);
                        locals.extend(used.0);
                    }
                }
            }
        }
        // Def-use worklist closes across loop backedges. Local granularity
        // conservatively retains all field/index writes and all definitions.
        let mut work: VecDeque<_> = locals.iter().copied().collect();
        while let Some(local) = work.pop_front() {
            for at in definitions.get(&local).into_iter().flatten() {
                if std::mem::replace(&mut statements[at.block.index()][at.statement_index], true) {
                    continue;
                }
                let mut used = Locals::default();
                used.visit_statement(
                    &body.basic_blocks[at.block].statements[at.statement_index],
                    *at,
                );
                for local in used.0 {
                    if locals.insert(local) {
                        work.push_back(local);
                    }
                }
            }
        }
        let stats = SliceStats {
            total_blocks: blocks.len(),
            retained_blocks: blocks.iter().filter(|b| **b).count(),
            total_statements: statements.iter().map(Vec::len).sum(),
            retained_statements: statements.iter().flatten().filter(|s| **s).count(),
            analysis_time: Default::default(),
        };
        Self {
            cfg,
            blocks,
            statements,
            locals,
            stats,
        }
    }
    pub fn contains(&self, block: rustc_middle::mir::BasicBlock) -> bool {
        self.blocks[block.index()]
    }
    pub fn transfers_terminator(&self, block: rustc_middle::mir::BasicBlock) -> bool {
        self.cfg.succ[block.index()]
            .iter()
            .any(|b| self.contains(*b))
    }
    pub fn retains(&self, at: Location) -> bool {
        self.statements[at.block.index()][at.statement_index]
    }
    pub fn tracks(&self, p: &Place<'_>) -> bool {
        self.locals.contains(&p.local)
    }
}
