use rustc_middle::mir::Place;
use std::collections::HashMap;
use std::collections::HashSet;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EqDomain<'tcx> {
    parent: HashMap<Place<'tcx>, Place<'tcx>>,
}

impl<'tcx> EqDomain<'tcx> {
    pub fn new() -> Self {
        Self {
            parent: HashMap::new(),
        }
    }

    fn ensure(&mut self, x: Place<'tcx>) {
        self.parent.entry(x).or_insert(x);
    }

    pub fn find(&mut self, x: Place<'tcx>) -> Place<'tcx> {
        self.ensure(x);
        let p = *self.parent.get(&x).unwrap();
        if p == x {
            return x;
        }
        let r = self.find(p);
        self.parent.insert(x, r);
        r
    }

    fn find_readonly(&self, x: Place<'tcx>) -> Place<'tcx> {
        let mut cur = x;
        loop {
            match self.parent.get(&cur).copied() {
                Some(p) if p != cur => cur = p,
                _ => return cur,
            }
        }
    }

    pub fn equiv(&mut self, a: Place<'tcx>, b: Place<'tcx>) -> bool {
        self.find(a) == self.find(b)
    }

    pub fn equiv_readonly(&self, a: Place<'tcx>, b: Place<'tcx>) -> bool {
        self.find_readonly(a) == self.find_readonly(b)
    }

    pub fn union(&mut self, a: Place<'tcx>, b: Place<'tcx>) {
        let ra = self.find(a);
        let rb = self.find(b);
        if ra != rb {
            self.parent.insert(ra, rb);
        }
    }

    pub fn kill(&mut self, x: Place<'tcx>) {
        self.ensure(x);
        let vars: Vec<Place<'tcx>> = self.parent.keys().copied().collect();
        let mut snapshot = self.clone();
        let class_members: Vec<Place<'tcx>> =
            vars.into_iter().filter(|&v| snapshot.equiv(v, x)).collect();

        for &v in &class_members {
            self.parent.insert(v, v);
        }

        let others: Vec<Place<'tcx>> = class_members.into_iter().filter(|&v| v != x).collect();
        if let Some(&head) = others.first() {
            for &v in others.iter().skip(1) {
                self.union(head, v);
            }
        }
    }

    pub fn equivalent_to(&self, other: &EqDomain<'tcx>) -> bool {
        let vars: Vec<Place<'tcx>> = {
            let mut s = HashSet::new();
            for &k in self.parent.keys() {
                s.insert(k);
            }
            for &k in other.parent.keys() {
                s.insert(k);
            }
            s.into_iter().collect()
        };

        let mut class_id_a: HashMap<Place<'tcx>, usize> = HashMap::new();
        let mut class_id_b: HashMap<Place<'tcx>, usize> = HashMap::new();
        let mut next_a = 0usize;
        let mut next_b = 0usize;
        let mut labels_a = Vec::with_capacity(vars.len());
        let mut labels_b = Vec::with_capacity(vars.len());

        for &v in &vars {
            let ra = self.find_readonly(v);
            let rb = other.find_readonly(v);
            let ida = *class_id_a.entry(ra).or_insert_with(|| {
                let id = next_a;
                next_a += 1;
                id
            });
            let idb = *class_id_b.entry(rb).or_insert_with(|| {
                let id = next_b;
                next_b += 1;
                id
            });
            labels_a.push(ida);
            labels_b.push(idb);
        }
        labels_a == labels_b
    }

    pub fn leq(&self, other: &EqDomain<'tcx>) -> bool {
        let vars: Vec<Place<'tcx>> = {
            let mut s = HashSet::new();
            for &k in self.parent.keys() {
                s.insert(k);
            }
            for &k in other.parent.keys() {
                s.insert(k);
            }
            s.into_iter().collect()
        };

        for i in 0..vars.len() {
            for j in (i + 1)..vars.len() {
                let x = vars[i];
                let y = vars[j];
                if other.equiv_readonly(x, y) && !self.equiv_readonly(x, y) {
                    return false;
                }
            }
        }
        true
    }
}

pub fn join_eq<'tcx>(a: &EqDomain<'tcx>, b: &EqDomain<'tcx>) -> EqDomain<'tcx> {
    let mut out = EqDomain::new();

    let vars: Vec<Place<'tcx>> = {
        let mut s = HashSet::new();
        for &k in a.parent.keys() {
            s.insert(k);
        }
        for &k in b.parent.keys() {
            s.insert(k);
        }
        s.into_iter().collect()
    };

    for &v in &vars {
        out.kill(v);
    }

    for i in 0..vars.len() {
        for j in (i + 1)..vars.len() {
            let x = vars[i];
            let y = vars[j];
            if a.equiv_readonly(x, y) && b.equiv_readonly(x, y) {
                out.union(x, y);
            }
        }
    }

    out
}
