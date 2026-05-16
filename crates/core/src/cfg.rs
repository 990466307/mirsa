use rustc_middle::mir::{BasicBlock, Body};

#[derive(Clone, Debug)]
pub struct Cfg {
    pub succ: Vec<Vec<BasicBlock>>,
    pub pred: Vec<Vec<BasicBlock>>,
}
pub fn build_cfg(body: &Body<'_>) -> Cfg {
    let n = body.basic_blocks.len();
    let mut succ = vec![Vec::<BasicBlock>::new(); n];
    let mut pred = vec![Vec::<BasicBlock>::new(); n];

    for (bb, bbdata) in body.basic_blocks.iter_enumerated() {
        let mut edges = Vec::new();
        if let Some(term) = &bbdata.terminator {
            term.successors()
                .filter(|s| !body.basic_blocks[*s].is_cleanup)
                .for_each(|s| edges.push(s));
        }
        succ[bb.index()] = edges.clone();
        for s in edges {
            pred[s.index()].push(bb);
        }
    }

    Cfg { succ, pred }
}
