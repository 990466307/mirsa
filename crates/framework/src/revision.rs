//! Mutation stamps for memoization; stamps are not part of abstract semantics.
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::hash::Hash;
use std::ops::{Deref, DerefMut};
use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Revision(u64);

impl Revision {
    fn fresh() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        Self(
            NEXT.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_add(1))
                .expect("abstract fact revision exhausted"),
        )
    }
}

/// Clones share a stamp until mutation. Unrestricted mutable access conservatively
/// advances it; common insertions preserve it when the fact is unchanged.
#[derive(Clone)]
pub struct Versioned<T> {
    value: T,
    revision: Revision,
}

impl<T> Versioned<T> {
    pub fn revision(&self) -> Revision {
        self.revision
    }
}
impl<T> From<T> for Versioned<T> {
    fn from(value: T) -> Self {
        Self {
            value,
            revision: Revision::fresh(),
        }
    }
}
impl<T: Default> Default for Versioned<T> {
    fn default() -> Self {
        T::default().into()
    }
}
impl<T: PartialEq> PartialEq for Versioned<T> {
    fn eq(&self, other: &Self) -> bool {
        self.value == other.value
    }
}
impl<T: Eq> Eq for Versioned<T> {}
impl<T: fmt::Debug> fmt::Debug for Versioned<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.value.fmt(f)
    }
}
impl<T> Deref for Versioned<T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.value
    }
}
impl<T> DerefMut for Versioned<T> {
    fn deref_mut(&mut self) -> &mut T {
        self.revision = Revision::fresh();
        &mut self.value
    }
}
impl<K: Eq + Hash, V: PartialEq> Versioned<HashMap<K, V>> {
    pub fn insert(&mut self, key: K, value: V) -> Option<V> {
        if self.value.get(&key) != Some(&value) {
            self.revision = Revision::fresh();
        }
        self.value.insert(key, value)
    }
    pub fn insert_if_absent(&mut self, key: K, value: V) {
        if let std::collections::hash_map::Entry::Vacant(entry) = self.value.entry(key) {
            entry.insert(value);
            self.revision = Revision::fresh();
        }
    }
}
impl<T: Eq + Hash> Versioned<HashSet<T>> {
    pub fn insert(&mut self, value: T) -> bool {
        let changed = self.value.insert(value);
        if changed {
            self.revision = Revision::fresh();
        }
        changed
    }
}
impl<A, T: FromIterator<A>> FromIterator<A> for Versioned<T> {
    fn from_iter<I: IntoIterator<Item = A>>(iter: I) -> Self {
        T::from_iter(iter).into()
    }
}
impl<T: IntoIterator> IntoIterator for Versioned<T> {
    type Item = T::Item;
    type IntoIter = T::IntoIter;
    fn into_iter(self) -> Self::IntoIter {
        self.value.into_iter()
    }
}
impl<'a, T> IntoIterator for &'a Versioned<T>
where
    &'a T: IntoIterator,
{
    type Item = <&'a T as IntoIterator>::Item;
    type IntoIter = <&'a T as IntoIterator>::IntoIter;
    fn into_iter(self) -> Self::IntoIter {
        (&self.value).into_iter()
    }
}
impl<'a, T> IntoIterator for &'a mut Versioned<T>
where
    &'a mut T: IntoIterator,
{
    type Item = <&'a mut T as IntoIterator>::Item;
    type IntoIter = <&'a mut T as IntoIterator>::IntoIter;
    fn into_iter(self) -> Self::IntoIter {
        self.deref_mut().into_iter()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn equivalent_writes_preserve_stamp_but_divergent_clones_do_not() {
        let mut original: Versioned<HashMap<u32, u32>> = HashMap::from([(1, 10)]).into();
        let stamp = original.revision();
        original.insert(1, 10);
        original.insert_if_absent(1, 20);
        assert_eq!(original.revision(), stamp);
        let mut other = original.clone();
        original.insert(1, 20);
        other.insert(1, 30);
        assert_ne!(original.revision(), stamp);
        assert_ne!(original.revision(), other.revision());
        other.insert(1, 20);
        assert_eq!(original, other); // History is not part of abstract equality.
        assert_ne!(original.revision(), other.revision());
    }

    #[test]
    fn unrestricted_mutation_cannot_bypass_revision() {
        let mut facts: Versioned<HashMap<u32, u32>> = HashMap::from([(1, 10)]).into();
        let before = facts.revision();
        *facts.get_mut(&1).unwrap() = 20;
        assert_ne!(facts.revision(), before);
        let before = facts.revision();
        facts.retain(|_, _| false);
        assert_ne!(facts.revision(), before);
        assert!(facts.is_empty());
    }
}
