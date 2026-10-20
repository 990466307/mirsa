use crate::revision::{Revision, Versioned};
use rustc_middle::mir::Place;
use std::collections::HashMap;
use std::collections::HashSet;
use std::hash::Hash;
use std::marker::PhantomData;

#[derive(Clone, Debug, Default)]
pub struct EqDomain<'tcx, K: Clone + Eq + Hash = Place<'tcx>> {
    parent: Versioned<HashMap<K, K>>,
    members: Versioned<HashMap<K, Vec<K>>>,
    _marker: PhantomData<&'tcx ()>,
}

impl<'tcx, K: Clone + Eq + Hash> PartialEq for EqDomain<'tcx, K> {
    fn eq(&self, other: &Self) -> bool {
        self.equivalent_to(other)
    }
}

impl<'tcx, K: Clone + Eq + Hash> Eq for EqDomain<'tcx, K> {}

impl<'tcx, K> EqDomain<'tcx, K>
where
    K: Clone + Eq + Hash,
{
    pub fn new() -> Self {
        Self {
            parent: HashMap::new().into(),
            members: HashMap::new().into(),
            _marker: PhantomData,
        }
    }

    fn ensure(&mut self, x: K) {
        if !self.parent.contains_key(&x) {
            self.parent.insert(x.clone(), x.clone());
            self.members.insert(x.clone(), vec![x]);
        }
    }

    pub fn find(&mut self, x: K) -> K {
        self.ensure(x.clone());
        self.find_readonly(x)
    }

    fn find_readonly(&self, x: K) -> K {
        self.parent.get(&x).cloned().unwrap_or(x)
    }

    pub fn members(&self, x: K) -> impl Iterator<Item = K> + '_ {
        let root = self.find_readonly(x.clone());
        let members = self.members.get(&root);
        members
            .into_iter()
            .flat_map(|members| members.iter().cloned())
            .chain(members.is_none().then_some(x))
    }

    pub fn equiv(&mut self, a: K, b: K) -> bool {
        self.find(a) == self.find(b)
    }

    pub fn equiv_readonly(&self, a: K, b: K) -> bool {
        self.find_readonly(a) == self.find_readonly(b)
    }

    pub fn union(&mut self, a: K, b: K) {
        let mut ra = self.find(a);
        let mut rb = self.find(b);
        if ra == rb {
            return;
        }
        if self.members[&ra].len() > self.members[&rb].len() {
            std::mem::swap(&mut ra, &mut rb);
        }
        let moved = self.members.remove(&ra).unwrap();
        for member in &moved {
            self.parent.insert(member.clone(), rb.clone());
        }
        self.members.get_mut(&rb).unwrap().extend(moved);
    }

    pub fn kill(&mut self, x: K) {
        self.ensure(x.clone());
        let root = self.find_readonly(x.clone());
        let mut members = self.members.remove(&root).unwrap();
        members.retain(|member| *member != x);
        if let Some(head) = members.first().cloned() {
            for member in &members {
                self.parent.insert(member.clone(), head.clone());
            }
            self.members.insert(head, members);
        }
        self.parent.insert(x.clone(), x.clone());
        self.members.insert(x.clone(), vec![x]);
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

    pub fn keys(&self) -> impl Iterator<Item = K> + '_ {
        self.parent.keys().cloned()
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

    // Two values remain equivalent precisely when both input classes agree.
    let mut classes: HashMap<(K, K), K> = HashMap::new();
    for value in vars {
        let pair = (
            a.find_readonly(value.clone()),
            b.find_readonly(value.clone()),
        );
        if let Some(first) = classes.get(&pair) {
            out.union(first.clone(), value);
        } else {
            classes.insert(pair, value);
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn intersection_retains_exactly_the_common_equivalences() {
        // All partitions of four values, including unequal input universes.
        let mut domains = Vec::new();
        for code in 0..256usize {
            let labels: Vec<_> = (0..4).map(|i| (code >> (i * 2)) & 3).collect();
            if labels[0] != 0 || (1..4).any(|i| labels[i] > 1 + *labels[..i].iter().max().unwrap())
            {
                continue;
            }
            let mut domain = EqDomain::new();
            for i in 0..4 {
                for j in 0..i {
                    if labels[i] == labels[j] {
                        domain.union(i, j);
                    }
                }
            }
            domains.push(domain);
        }
        assert_eq!(domains.len(), 15);
        for left in &domains {
            for right in &domains {
                let joined = join_eq(left, right);
                for i in 0..5 {
                    for j in 0..5 {
                        assert_eq!(
                            joined.equiv_readonly(i, j),
                            left.equiv_readonly(i, j) && right.equiv_readonly(i, j)
                        );
                    }
                }
            }
        }
    }
}

impl<'tcx, K: Clone + Eq + Hash> EqDomain<'tcx, K> {
    pub fn revision(&self) -> [Revision; 2] {
        [self.parent.revision(), self.members.revision()]
    }
}
