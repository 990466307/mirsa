use crate::access_path::{AccessPath, AccessPathElem};
use rustc_middle::mir::{Operand, Place};
use std::collections::{HashMap, HashSet};

pub fn update_value_children<T>(
    values: &mut HashMap<AccessPath, T>,
    root: &AccessPath,
    mut value: impl FnMut(&[AccessPathElem]) -> T,
) {
    for (path, current) in values {
        if let Some(suffix) = path.value_suffix(root).filter(|s| !s.is_empty()) {
            *current = value(suffix);
        }
    }
}

pub fn subtree_pairs<'a>(
    destination: &AccessPath,
    source: &AccessPath,
    paths: impl IntoIterator<Item = &'a AccessPath>,
    include_root: bool,
) -> Vec<(AccessPath, AccessPath)> {
    let mut suffixes = HashSet::new();
    if include_root {
        suffixes.insert(Vec::new());
    }
    for path in paths {
        for root in [destination, source] {
            if let Some(suffix) = path.strip_pattern_prefix(root) {
                if include_root || !suffix.is_empty() {
                    suffixes.insert(suffix);
                }
            }
        }
    }
    suffixes
        .into_iter()
        .map(|suffix| {
            (
                destination.join_suffix(&suffix),
                source.join_suffix(&suffix),
            )
        })
        .collect()
}

pub trait PathMapping<'tcx> {
    fn path(&self, path: &AccessPath) -> Option<AccessPath>;
    fn place(&self, place: Place<'tcx>) -> Option<Place<'tcx>>;

    fn operand(&self, operand: &Operand<'tcx>) -> Option<Operand<'tcx>> {
        Some(match operand {
            Operand::Copy(place) => Operand::Copy(self.place(*place)?),
            Operand::Move(place) => Operand::Copy(self.place(*place)?),
            Operand::Constant(value) => Operand::Constant(value.clone()),
        })
    }

    fn values<T: Clone>(&self, values: &HashMap<AccessPath, T>) -> HashMap<AccessPath, T> {
        values
            .iter()
            .filter_map(|(path, value)| Some((self.path(path)?, value.clone())))
            .collect()
    }

    fn paths(&self, paths: &HashSet<AccessPath>) -> HashSet<AccessPath> {
        paths.iter().filter_map(|path| self.path(path)).collect()
    }

    fn places(
        &self,
        places: &HashMap<AccessPath, Place<'tcx>>,
    ) -> HashMap<AccessPath, Place<'tcx>> {
        places
            .iter()
            .filter_map(|(path, place)| Some((self.path(path)?, self.place(*place)?)))
            .collect()
    }
}
