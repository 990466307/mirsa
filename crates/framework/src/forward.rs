use std::collections::{HashMap, VecDeque};

use mirsa_core::cfg::Cfg;
use rustc_middle::mir::{
    BasicBlock, Body, LocalDecls, Location, START_BLOCK, Statement, Terminator,
};
use rustc_middle::ty::TyCtxt;

#[derive(Clone, Copy, Debug)]
pub struct PathForwardAnalysisConfig {
    pub max_paths: usize,
    pub widen_after_iterations: Option<u32>,
}

#[derive(Clone, Debug)]
pub struct PathForwardAnalysisResult<S> {
    pub in_states: Vec<S>,
    pub out_states: Vec<S>,
    pub reachable: Vec<bool>,
}

pub trait DomainState<'tcx>: Clone + PartialEq {
    fn join(left: &Self, right: &Self) -> Self;

    fn widen(_previous: &Self, next: &Self) -> Self {
        next.clone()
    }

    fn state_changed(previous: &Self, next: &Self) -> bool {
        previous != next
    }
}

pub trait ForwardSemantics<'tcx> {
    type State: DomainState<'tcx>;

    fn bottom(&self, body: &Body<'tcx>) -> Self::State;

    fn entry_state(&self, body: &Body<'tcx>) -> Self::State {
        self.bottom(body)
    }

    fn transfer_stmt(
        &self,
        tcx: TyCtxt<'tcx>,
        st: &mut Self::State,
        stmt: &Statement<'tcx>,
        local_decls: &LocalDecls<'tcx>,
    );

    fn transfer_terminator(
        &self,
        _tcx: TyCtxt<'tcx>,
        _st: &mut Self::State,
        _term: &Terminator<'tcx>,
        _local_decls: &LocalDecls<'tcx>,
    ) {
    }

    fn transfer_block(
        &self,
        tcx: TyCtxt<'tcx>,
        body: &Body<'tcx>,
        bb: BasicBlock,
        in_state: &Self::State,
    ) -> Self::State {
        let mut st = in_state.clone();
        let data = &body.basic_blocks[bb];
        for stmt in &data.statements {
            self.transfer_stmt(tcx, &mut st, stmt, &body.local_decls);
        }
        if let Some(term) = &data.terminator {
            self.transfer_terminator(tcx, &mut st, term, &body.local_decls);
        }
        st
    }

    fn refine_edge(
        &self,
        _tcx: TyCtxt<'tcx>,
        _body: &Body<'tcx>,
        _pred: BasicBlock,
        _succ: BasicBlock,
        in_state: &Self::State,
    ) -> Option<Self::State> {
        Some(in_state.clone())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct PerPathState<S> {
    in_states: Vec<S>,
    out_states: Vec<S>,
    iterations: Vec<u32>,
    visited: Vec<bool>,
    is_abstract: bool,
}

#[derive(Clone, Copy, Debug)]
struct PathWorkItem {
    pred: Option<BasicBlock>,
    bb: BasicBlock,
    path_id: u32,
}

pub fn run_path_sensitive_forward_analysis_with_config<'tcx, A>(
    tcx: TyCtxt<'tcx>,
    body: &Body<'tcx>,
    cfg: &Cfg,
    semantics: &A,
    config: PathForwardAnalysisConfig,
) -> PathForwardAnalysisResult<A::State>
where
    A: ForwardSemantics<'tcx>,
{
    let n = body.basic_blocks.len();
    let bottom = semantics.bottom(body);
    let entry = semantics.entry_state(body);
    let mut worklist = VecDeque::new();
    worklist.push_back(PathWorkItem {
        pred: None,
        bb: START_BLOCK,
        path_id: 0,
    });

    let mut path_id_seed = 0u32;
    let mut reached_max_paths = false;

    let mut path_map: HashMap<u32, PerPathState<A::State>> = HashMap::new();
    path_map.insert(
        0,
        PerPathState {
            in_states: vec![bottom.clone(); n],
            out_states: vec![bottom.clone(); n],
            iterations: vec![0; n],
            visited: vec![false; n],
            is_abstract: false,
        },
    );

    while let Some(item) = worklist.pop_front() {
        let in_state = {
            let current = path_map.get(&item.path_id).unwrap();
            if current.is_abstract {
                let mut merged = bottom.clone();
                for pred in &cfg.pred[item.bb.index()] {
                    let pred_state = &current.out_states[pred.index()];
                    if let Some(refined) =
                        semantics.refine_edge(tcx, body, *pred, item.bb, pred_state)
                    {
                        merged = A::State::join(&merged, &refined);
                    }
                }
                merged
            } else {
                match item.pred {
                    None => entry.clone(),
                    Some(pred) => {
                        let pred_state = &current.out_states[pred.index()];
                        let Some(refined) =
                            semantics.refine_edge(tcx, body, pred, item.bb, pred_state)
                        else {
                            path_map.remove(&item.path_id);
                            continue;
                        };
                        refined
                    }
                }
            }
        };

        let raw_out = semantics.transfer_block(tcx, body, item.bb, &in_state);
        let path_prev = path_map.get(&item.path_id).unwrap().out_states[item.bb.index()].clone();
        let first_visit = !path_map.get(&item.path_id).unwrap().visited[item.bb.index()];
        let do_widen = config.widen_after_iterations.is_some_and(|limit| {
            path_map.get(&item.path_id).unwrap().iterations[item.bb.index()] >= limit
        });
        let real_out = if do_widen {
            A::State::widen(&path_prev, &raw_out)
        } else {
            raw_out
        };
        let state_changed = A::State::state_changed(&path_prev, &real_out);

        {
            let current = path_map.get_mut(&item.path_id).unwrap();
            current.in_states[item.bb.index()] = in_state;
            current.out_states[item.bb.index()] = real_out;
            current.iterations[item.bb.index()] += 1;
            current.visited[item.bb.index()] = true;
        }

        if first_visit || state_changed {
            let parent = path_map.get(&item.path_id).unwrap().clone();
            let mut succ_num = 0usize;
            for succ in &cfg.succ[item.bb.index()] {
                succ_num += 1;
                if succ_num == 1 {
                    worklist.push_back(PathWorkItem {
                        pred: Some(item.bb),
                        bb: *succ,
                        path_id: item.path_id,
                    });
                    continue;
                }

                if path_map.len() >= config.max_paths {
                    reached_max_paths = true;
                }
                if !reached_max_paths {
                    path_id_seed += 1;
                    let new_path_id = path_id_seed;
                    path_map.insert(new_path_id, parent.clone());
                    worklist.push_back(PathWorkItem {
                        pred: Some(item.bb),
                        bb: *succ,
                        path_id: new_path_id,
                    });
                } else {
                    worklist.push_back(PathWorkItem {
                        pred: Some(item.bb),
                        bb: *succ,
                        path_id: item.path_id,
                    });
                    if let Some(current) = path_map.get_mut(&item.path_id) {
                        current.is_abstract = true;
                    }
                }
            }
        }
    }

    let mut final_in_states = Vec::with_capacity(n);
    let mut final_out_states = Vec::with_capacity(n);
    let mut reachable = Vec::with_capacity(n);
    for bb in body.basic_blocks.indices() {
        let mut merged_in = bottom.clone();
        let mut merged_out = bottom.clone();
        let mut reached = false;
        for path in path_map.values() {
            reached |= path.visited[bb.index()];
            merged_in = A::State::join(&merged_in, &path.in_states[bb.index()]);
            merged_out = A::State::join(&merged_out, &path.out_states[bb.index()]);
        }
        final_in_states.push(merged_in);
        final_out_states.push(merged_out);
        reachable.push(reached);
    }

    let _ = reached_max_paths;
    PathForwardAnalysisResult {
        in_states: final_in_states,
        out_states: final_out_states,
        reachable,
    }
}

pub fn replay_state_before_location<'tcx, S>(
    tcx: TyCtxt<'tcx>,
    body: &Body<'tcx>,
    in_state: &S,
    location: Location,
    mut transfer_stmt: impl FnMut(TyCtxt<'tcx>, &mut S, &Statement<'tcx>, &LocalDecls<'tcx>),
) -> Option<S>
where
    S: Clone,
{
    let bbdata = &body.basic_blocks[location.block];
    if location.statement_index > bbdata.statements.len() {
        return None;
    }

    let mut state = in_state.clone();
    for stmt in bbdata.statements.iter().take(location.statement_index) {
        transfer_stmt(tcx, &mut state, stmt, &body.local_decls);
    }
    Some(state)
}

pub fn state_before_location_from_result<'tcx, S>(
    tcx: TyCtxt<'tcx>,
    body: &Body<'tcx>,
    result: &PathForwardAnalysisResult<S>,
    location: Location,
    transfer_stmt: impl FnMut(TyCtxt<'tcx>, &mut S, &Statement<'tcx>, &LocalDecls<'tcx>),
) -> Option<S>
where
    S: Clone,
{
    if !result
        .reachable
        .get(location.block.index())
        .copied()
        .unwrap_or(false)
    {
        return None;
    }
    let in_state = result.in_states.get(location.block.index())?;
    replay_state_before_location(tcx, body, in_state, location, transfer_stmt)
}
