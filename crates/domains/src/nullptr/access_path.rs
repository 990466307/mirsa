use rustc_middle::mir::{Local, ProjectionElem};
use std::fmt;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct AccessPath {
    pub root: Local,
    pub elems: Vec<AccessPathElem>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum AccessPathElem {
    Deref,
    Field(usize),
    Index,
    ConstantIndex { offset: u64, from_end: bool },
}

impl AccessPath {
    pub fn from_local(local: Local) -> Self {
        Self {
            root: local,
            elems: Vec::new(),
        }
    }

    pub fn from_projection<'tcx>(
        mut root: AccessPath,
        projection: &'tcx [ProjectionElem<Local, rustc_middle::ty::Ty<'tcx>>],
    ) -> Option<Self> {
        for elem in projection {
            match elem {
                ProjectionElem::Deref => root.push(AccessPathElem::Deref),
                ProjectionElem::Field(field, _) => {
                    root.push(AccessPathElem::Field(field.as_usize()))
                }
                ProjectionElem::Index(_) => root.push(AccessPathElem::Index),
                ProjectionElem::ConstantIndex {
                    offset, from_end, ..
                } => root.push(AccessPathElem::ConstantIndex {
                    offset: *offset,
                    from_end: *from_end,
                }),
                _ => return None,
            }
        }
        Some(root)
    }

    pub fn deref(&self) -> Self {
        let mut out = self.clone();
        out.push(AccessPathElem::Deref);
        out
    }

    pub fn join_suffix(&self, suffix: &[AccessPathElem]) -> Self {
        let mut out = self.clone();
        for elem in suffix {
            out.push(elem.clone());
        }
        out
    }

    pub fn strip_prefix(&self, prefix: &AccessPath) -> Option<Vec<AccessPathElem>> {
        if self.root != prefix.root || self.elems.len() < prefix.elems.len() {
            return None;
        }
        if self.elems[..prefix.elems.len()] != prefix.elems {
            return None;
        }
        Some(self.elems[prefix.elems.len()..].to_vec())
    }

    pub fn strip_pattern_prefix(&self, prefix: &AccessPath) -> Option<Vec<AccessPathElem>> {
        if self.root != prefix.root || self.elems.len() < prefix.elems.len() {
            return None;
        }
        for (actual, expected) in self.elems.iter().zip(prefix.elems.iter()) {
            if !elem_matches(actual, expected) {
                return None;
            }
        }
        Some(self.elems[prefix.elems.len()..].to_vec())
    }

    pub fn matches_pattern(&self, pattern: &AccessPath) -> bool {
        self.root == pattern.root
            && self.elems.len() == pattern.elems.len()
            && self
                .elems
                .iter()
                .zip(pattern.elems.iter())
                .all(|(actual, expected)| elem_matches(actual, expected))
    }

    pub fn has_index(&self) -> bool {
        self.elems
            .iter()
            .any(|elem| matches!(elem, AccessPathElem::Index))
    }

    fn push(&mut self, elem: AccessPathElem) {
        self.elems.push(elem);
    }
}

fn elem_matches(actual: &AccessPathElem, pattern: &AccessPathElem) -> bool {
    match pattern {
        AccessPathElem::Index => {
            matches!(
                actual,
                AccessPathElem::Index | AccessPathElem::ConstantIndex { .. }
            )
        }
        _ => actual == pattern,
    }
}

impl fmt::Display for AccessPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self.root)?;
        for elem in &self.elems {
            match elem {
                AccessPathElem::Deref => write!(f, ".*")?,
                AccessPathElem::Field(field) => write!(f, ".{field}")?,
                AccessPathElem::Index => write!(f, "[_]")?,
                AccessPathElem::ConstantIndex {
                    offset,
                    from_end: false,
                } => write!(f, "[{offset}]")?,
                AccessPathElem::ConstantIndex {
                    offset,
                    from_end: true,
                } => write!(f, "[-{offset}]")?,
            }
        }
        Ok(())
    }
}
