use super::PathWorkItem;
use std::collections::VecDeque;

pub(super) struct PathWorklist<S> {
    items: VecDeque<PathWorkItem<S>>,
    pending: Vec<Vec<usize>>,
}

impl<S> PathWorklist<S> {
    pub(super) fn new(blocks: usize) -> Self {
        Self {
            items: VecDeque::new(),
            pending: vec![vec![0; blocks]],
        }
    }

    pub(super) fn add_path(&mut self, path_id: u32) {
        assert_eq!(path_id as usize, self.pending.len());
        self.pending.push(vec![0; self.pending[0].len()]);
    }

    pub(super) fn push(&mut self, item: PathWorkItem<S>, merged: bool) {
        let count = &mut self.pending[item.path_id as usize][item.bb.index()];
        // Only merged paths read all predecessors when dequeued. Concrete
        // paths must retain each incoming state, even for the same block.
        if merged && *count != 0 {
            return;
        }
        *count += 1;
        self.items.push_back(item);
    }

    pub(super) fn pop(&mut self) -> Option<PathWorkItem<S>> {
        let item = self.items.pop_front()?;
        // A path may have become merged while concrete items were pending.
        self.pending[item.path_id as usize][item.bb.index()] -= 1;
        Some(item)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustc_middle::mir::BasicBlock;

    fn item(path_id: u32, pred: usize) -> PathWorkItem<usize> {
        PathWorkItem {
            bb: BasicBlock::from_usize(2),
            path_id,
            incoming: Some(pred),
        }
    }

    #[test]
    fn concrete_predecessors_survive_path_promotion() {
        let mut queue = PathWorklist::new(3);
        queue.push(item(0, 0), false);
        queue.push(item(0, 1), false);
        queue.push(item(0, 0), true);
        assert_eq!(queue.pop().unwrap().incoming, Some(0));
        queue.push(item(0, 0), true);
        assert_eq!(queue.pop().unwrap().incoming, Some(1));
        assert!(queue.pop().is_none());
    }

    #[test]
    fn merged_item_can_be_rescheduled_after_dequeue() {
        let mut queue = PathWorklist::new(3);
        queue.push(item(0, 0), true);
        queue.push(item(0, 1), true);
        assert!(queue.pop().is_some());
        queue.push(item(0, 1), true);
        assert!(queue.pop().is_some());
        assert!(queue.pop().is_none());
    }

    #[test]
    fn paths_have_independent_pending_items() {
        let mut queue = PathWorklist::new(3);
        queue.push(item(0, 0), true);
        queue.add_path(1);
        queue.push(item(1, 1), true);
        assert_eq!(queue.pop().unwrap().path_id, 0);
        assert_eq!(queue.pop().unwrap().path_id, 1);
        assert!(queue.pop().is_none());
    }
}
