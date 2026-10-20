//! Selection of stored facts related to requested access paths.
use crate::access_path::AccessPath;

/// Select a value and its stored fields, but do not follow references recursively.
pub fn selected(path: &AccessPath, roots: &[AccessPath]) -> bool {
    roots.iter().any(|root| path.strip_prefix(root).is_some())
}
