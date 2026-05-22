use rustc_middle::mir::Place;
use std::collections::HashMap;
use std::collections::HashSet;
use std::hash::Hash;
use std::marker::PhantomData;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EqDomain<'tcx, K: Clone + Eq + Hash = Place<'tcx>> {
    parent: HashMap<K, K>,
    _marker: PhantomData<&'tcx ()>,
}

impl<'tcx, K> EqDomain<'tcx, K>
where
    K: Clone + Eq + Hash,
{
    pub fn new() -> Self {
        Self {
            parent: HashMap::new(),
            _marker: PhantomData,
        }
    }

    fn ensure(&mut self, x: K) {
        self.parent.entry(x.clone()).or_insert(x);
    }

    pub fn find(&mut self, x: K) -> K {
        self.ensure(x.clone());
        let p = self.parent.get(&x).unwrap().clone();
        if p == x {
            return x;
        }
        let r = self.find(p);
        self.parent.insert(x, r.clone());
        r
    }

    fn find_readonly(&self, x: K) -> K {
        let mut cur = x;
        loop {
            match self.parent.get(&cur).cloned() {
                Some(p) if p != cur => cur = p,
                _ => return cur,
            }
        }
    }

    pub fn equiv(&mut self, a: K, b: K) -> bool {
        self.find(a) == self.find(b)
    }

    pub fn equiv_readonly(&self, a: K, b: K) -> bool {
        self.find_readonly(a) == self.find_readonly(b)
    }

    pub fn union(&mut self, a: K, b: K) {
        let ra = self.find(a);
        let rb = self.find(b);
        if ra != rb {
            self.parent.insert(ra, rb);
        }
    }

    pub fn kill(&mut self, x: K) {
        self.ensure(x.clone());
        let vars: Vec<K> = self.parent.keys().cloned().collect();
        let mut snapshot = self.clone();
        let class_members: Vec<K> = vars
            .into_iter()
            .filter(|v| snapshot.equiv(v.clone(), x.clone()))
            .collect();

        for v in &class_members {
            self.parent.insert(v.clone(), v.clone());
        }

        let others: Vec<K> = class_members.into_iter().filter(|v| *v != x).collect();
        if let Some(head) = others.first() {
            for v in others.iter().skip(1) {
                self.union(head.clone(), v.clone());
            }
        }
    }

    pub fn equivalent_to(&self, other: &EqDomain<'tcx, K>) -> bool {
        let vars: Vec<K> = {
            let mut s = HashSet::new();
            for k in self.parent.keys() {
                s.insert(k.clone());
            }
            for k in other.parent.keys() {
                s.insert(k.clone());
            }
            s.into_iter().collect()
        };

        let mut class_id_a: HashMap<K, usize> = HashMap::new();
        let mut class_id_b: HashMap<K, usize> = HashMap::new();
        let mut next_a = 0usize;
        let mut next_b = 0usize;
        let mut labels_a = Vec::with_capacity(vars.len());
        let mut labels_b = Vec::with_capacity(vars.len());

        for v in &vars {
            let ra = self.find_readonly(v.clone());
            let rb = other.find_readonly(v.clone());
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

    pub fn leq(&self, other: &EqDomain<'tcx, K>) -> bool {
        let vars: Vec<K> = {
            let mut s = HashSet::new();
            for k in self.parent.keys() {
                s.insert(k.clone());
            }
            for k in other.parent.keys() {
                s.insert(k.clone());
            }
            s.into_iter().collect()
        };

        for i in 0..vars.len() {
            for j in (i + 1)..vars.len() {
                let x = vars[i].clone();
                let y = vars[j].clone();
                if other.equiv_readonly(x.clone(), y.clone()) && !self.equiv_readonly(x, y) {
                    return false;
                }
            }
        }
        true
    }
}

pub fn join_eq<'tcx, K>(a: &EqDomain<'tcx, K>, b: &EqDomain<'tcx, K>) -> EqDomain<'tcx, K>
where
    K: Clone + Eq + Hash,
{
    let mut out = EqDomain::new();

    let vars: Vec<K> = {
        let mut s = HashSet::new();
        for k in a.parent.keys() {
            s.insert(k.clone());
        }
        for k in b.parent.keys() {
            s.insert(k.clone());
        }
        s.into_iter().collect()
    };

    for v in &vars {
        out.kill(v.clone());
    }

    for i in 0..vars.len() {
        for j in (i + 1)..vars.len() {
            let x = vars[i].clone();
            let y = vars[j].clone();
            if a.equiv_readonly(x.clone(), y.clone()) && b.equiv_readonly(x.clone(), y.clone()) {
                out.union(x, y);
            }
        }
    }

    out
}
