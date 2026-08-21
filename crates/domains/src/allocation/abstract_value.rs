use crate::interval::abstract_value::{Interval, intersect, join as join_interval, widen};
use mirsa_framework::access_path::AccessPath;
use rustc_hir::def_id::DefId;
use rustc_middle::mir::Local;
use rustc_span::Span;
use std::collections::HashMap;
use std::fmt;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct AllocationSite {
    pub span: Span,
    pub destination: AccessPath,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum AllocationId {
    Stack(Local),
    Heap(AllocationSite),
    Static(DefId),
    External(Local),
}

impl AllocationId {
    pub fn origin(&self) -> AllocationOrigin {
        match self {
            Self::Stack(_) => AllocationOrigin::Stack,
            Self::Heap(_) => AllocationOrigin::Heap,
            Self::Static(_) => AllocationOrigin::Static,
            Self::External(_) => AllocationOrigin::External,
        }
    }

    /// Different external arguments can still denote the same caller-owned
    /// allocation. Other distinct abstract objects are disjoint.
    pub fn may_alias(&self, other: &Self) -> bool {
        self == other || matches!((self, other), (Self::External(_), Self::External(_)))
    }
}

impl fmt::Display for AllocationId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Stack(local) => write!(f, "stack({local:?})"),
            Self::Heap(site) => write!(f, "heap({:?}, {})", site.span, site.destination),
            Self::Static(def_id) => write!(f, "static({def_id:?})"),
            Self::External(local) => write!(f, "external({local:?})"),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AllocationOrigin {
    Stack,
    Heap,
    Static,
    External,
    Unknown,
}

impl fmt::Display for AllocationOrigin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self)
    }
}

/// Powerset abstraction of the concrete allocation states. `ABSENT` is
/// needed for allocation sites which are reached on only some paths.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct AllocationStatus(u8);

impl AllocationStatus {
    const ABSENT: u8 = 1;
    const LIVE: u8 = 2;
    const DEAD: u8 = 4;

    pub const fn bottom() -> Self {
        Self(0)
    }

    pub const fn absent() -> Self {
        Self(Self::ABSENT)
    }

    pub const fn live() -> Self {
        Self(Self::LIVE)
    }

    pub const fn dead() -> Self {
        Self(Self::DEAD)
    }

    pub const fn absent_or_live() -> Self {
        Self(Self::ABSENT | Self::LIVE)
    }

    pub const fn top() -> Self {
        Self(Self::ABSENT | Self::LIVE | Self::DEAD)
    }

    pub const fn is_bottom(self) -> bool {
        self.0 == 0
    }

    pub const fn may_be_absent(self) -> bool {
        self.0 & Self::ABSENT != 0
    }

    pub const fn may_be_live(self) -> bool {
        self.0 & Self::LIVE != 0
    }

    pub const fn may_be_dead(self) -> bool {
        self.0 & Self::DEAD != 0
    }

    pub const fn is_definitely_live(self) -> bool {
        self.0 == Self::LIVE
    }

    pub const fn is_definitely_dead(self) -> bool {
        self.0 == Self::DEAD
    }

    pub const fn join(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    pub const fn intersect(self, other: Self) -> Self {
        Self(self.0 & other.0)
    }
}

impl fmt::Display for AllocationStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_bottom() {
            return write!(f, "Bot");
        }
        let mut names = Vec::new();
        if self.may_be_absent() {
            names.push("Absent");
        }
        if self.may_be_live() {
            names.push("Live");
        }
        if self.may_be_dead() {
            names.push("Dead");
        }
        write!(f, "{}", names.join("|"))
    }
}

/// Powerset abstraction of the number of concrete objects summarized by an
/// allocation id. It prevents strong updates once a loop has allocated at the
/// same site more than once.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct AllocationMultiplicity(u8);

impl AllocationMultiplicity {
    const ZERO: u8 = 1;
    const ONE: u8 = 2;
    const MANY: u8 = 4;

    pub const fn zero() -> Self {
        Self(Self::ZERO)
    }

    pub const fn one() -> Self {
        Self(Self::ONE)
    }

    pub const fn join(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    pub const fn is_exactly_one(self) -> bool {
        self.0 == Self::ONE
    }

    pub fn after_allocation(self) -> Self {
        let mut bits = 0;
        if self.0 & Self::ZERO != 0 {
            bits |= Self::ONE;
        }
        if self.0 & (Self::ONE | Self::MANY) != 0 {
            bits |= Self::MANY;
        }
        Self(bits)
    }
}

impl fmt::Display for AllocationMultiplicity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut names = Vec::new();
        if self.0 & Self::ZERO != 0 {
            names.push("0");
        }
        if self.0 & Self::ONE != 0 {
            names.push("1");
        }
        if self.0 & Self::MANY != 0 {
            names.push("many");
        }
        write!(f, "{}", names.join("|"))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AllocationFact {
    pub origin: AllocationOrigin,
    pub status: AllocationStatus,
    pub extent: Interval,
    pub multiplicity: AllocationMultiplicity,
}

impl AllocationFact {
    pub fn absent(origin: AllocationOrigin) -> Self {
        Self {
            origin,
            status: AllocationStatus::absent(),
            extent: Interval::empty(),
            multiplicity: AllocationMultiplicity::zero(),
        }
    }

    pub fn live(origin: AllocationOrigin, extent: Interval) -> Self {
        Self {
            origin,
            status: AllocationStatus::live(),
            extent,
            multiplicity: AllocationMultiplicity::one(),
        }
    }

    pub fn join(left: &Self, right: &Self) -> Self {
        Self {
            origin: if left.origin == right.origin {
                left.origin
            } else {
                AllocationOrigin::Unknown
            },
            status: left.status.join(right.status),
            extent: join_interval(&left.extent, &right.extent),
            multiplicity: left.multiplicity.join(right.multiplicity),
        }
    }

    pub fn widen(previous: &Self, next: &Self) -> Self {
        Self {
            origin: if previous.origin == next.origin {
                previous.origin
            } else {
                AllocationOrigin::Unknown
            },
            status: previous.status.join(next.status),
            extent: widen(&previous.extent, &next.extent),
            multiplicity: previous.multiplicity.join(next.multiplicity),
        }
    }
}

impl fmt::Display for AllocationFact {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} {} bytes={} count={}",
            self.origin, self.status, self.extent, self.multiplicity
        )
    }
}

/// An allocation target is kept together with its byte offset. The additional
/// flags distinguish null and known-invalid pointers from pointers whose
/// allocation identity is simply unknown.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PointerValue {
    targets: HashMap<AllocationId, Interval>,
    may_be_null: bool,
    may_be_invalid_non_null: bool,
    unknown: bool,
}

impl PointerValue {
    pub fn bottom() -> Self {
        Self {
            targets: HashMap::new(),
            may_be_null: false,
            may_be_invalid_non_null: false,
            unknown: false,
        }
    }

    pub fn top() -> Self {
        Self {
            targets: HashMap::new(),
            may_be_null: true,
            may_be_invalid_non_null: true,
            unknown: true,
        }
    }

    pub fn unknown_non_null() -> Self {
        Self {
            targets: HashMap::new(),
            may_be_null: false,
            may_be_invalid_non_null: true,
            unknown: true,
        }
    }

    pub fn null() -> Self {
        Self {
            targets: HashMap::new(),
            may_be_null: true,
            may_be_invalid_non_null: false,
            unknown: false,
        }
    }

    pub fn invalid_non_null() -> Self {
        Self {
            targets: HashMap::new(),
            may_be_null: false,
            may_be_invalid_non_null: true,
            unknown: false,
        }
    }

    pub fn target(id: AllocationId, offset: Interval) -> Self {
        Self {
            targets: HashMap::from([(id, offset)]),
            may_be_null: false,
            may_be_invalid_non_null: false,
            unknown: false,
        }
    }

    pub fn fallible_target(id: AllocationId, offset: Interval) -> Self {
        let mut out = Self::target(id, offset);
        out.may_be_null = true;
        out
    }

    pub fn is_bottom(&self) -> bool {
        !self.unknown
            && !self.may_be_null
            && !self.may_be_invalid_non_null
            && self.targets.is_empty()
    }

    pub fn is_top(&self) -> bool {
        self.unknown && self.may_be_null && self.may_be_invalid_non_null
    }

    pub fn may_be_null(&self) -> bool {
        self.may_be_null
    }

    pub fn may_be_invalid_non_null(&self) -> bool {
        self.may_be_invalid_non_null
    }

    pub fn targets(&self) -> impl Iterator<Item = (&AllocationId, &Interval)> {
        self.targets.iter()
    }

    pub fn target_count(&self) -> usize {
        self.targets.len()
    }

    pub fn has_unknown_target(&self) -> bool {
        self.unknown
    }

    pub fn is_null_only(&self) -> bool {
        !self.unknown
            && self.may_be_null
            && !self.may_be_invalid_non_null
            && self.targets.is_empty()
    }

    pub fn is_definitely_non_null(&self) -> bool {
        !self.may_be_null && !self.is_bottom()
    }

    pub fn exact_target(&self) -> Option<(&AllocationId, Interval)> {
        if self.unknown
            || self.may_be_null
            || self.may_be_invalid_non_null
            || self.targets.len() != 1
        {
            return None;
        }
        self.targets.iter().next().map(|(id, offset)| (id, *offset))
    }

    pub fn join(left: &Self, right: &Self) -> Self {
        if left.is_bottom() {
            return right.clone();
        }
        if right.is_bottom() {
            return left.clone();
        }
        let mut out = Self {
            targets: left.targets.clone(),
            may_be_null: left.may_be_null || right.may_be_null,
            may_be_invalid_non_null: left.may_be_invalid_non_null || right.may_be_invalid_non_null,
            unknown: left.unknown || right.unknown,
        };
        for (id, offset) in &right.targets {
            out.targets
                .entry(id.clone())
                .and_modify(|old| *old = join_interval(old, offset))
                .or_insert(*offset);
        }
        out
    }

    pub fn widen(previous: &Self, next: &Self) -> Self {
        if previous.is_bottom() {
            return next.clone();
        }
        if next.is_bottom() {
            return previous.clone();
        }
        let mut out = Self {
            targets: HashMap::new(),
            may_be_null: previous.may_be_null || next.may_be_null,
            may_be_invalid_non_null: previous.may_be_invalid_non_null
                || next.may_be_invalid_non_null,
            unknown: previous.unknown || next.unknown,
        };
        for id in previous.targets.keys().chain(next.targets.keys()) {
            let value = match (previous.targets.get(id), next.targets.get(id)) {
                (Some(old), Some(new)) => widen(old, new),
                (Some(value), None) | (None, Some(value)) => *value,
                (None, None) => unreachable!(),
            };
            out.targets.insert(id.clone(), value);
        }
        out
    }

    pub fn add_offset(&self, delta: Interval) -> Self {
        let mut out = self.clone();
        for offset in out.targets.values_mut() {
            *offset = crate::interval::abstract_value::add(offset, &delta);
        }
        out
    }

    pub fn forget_offsets(&self) -> Self {
        let mut out = self.clone();
        for offset in out.targets.values_mut() {
            *offset = Interval::top();
        }
        out
    }

    pub fn without_null(&self) -> Self {
        let mut out = self.clone();
        out.may_be_null = false;
        out
    }

    pub fn only_null(&self) -> Self {
        if self.may_be_null() {
            Self::null()
        } else {
            Self::bottom()
        }
    }

    pub fn intersect_equal(left: &Self, right: &Self) -> Self {
        if left.is_bottom() || right.is_bottom() {
            return Self::bottom();
        }
        let mut out = Self::bottom();
        out.may_be_null = left.may_be_null && right.may_be_null;
        out.may_be_invalid_non_null = left.may_be_invalid_non_null && right.may_be_invalid_non_null;
        out.unknown = left.unknown && right.unknown;
        for (id, left_offset) in &left.targets {
            let Some(right_offset) = right.targets.get(id) else {
                continue;
            };
            let common = intersect(left_offset, right_offset);
            if !common.is_empty() {
                out.targets.insert(id.clone(), common);
            }
        }
        if left.unknown {
            for (id, offset) in &right.targets {
                out.targets
                    .entry(id.clone())
                    .and_modify(|old| *old = join_interval(old, offset))
                    .or_insert(*offset);
            }
        }
        if right.unknown {
            for (id, offset) in &left.targets {
                out.targets
                    .entry(id.clone())
                    .and_modify(|old| *old = join_interval(old, offset))
                    .or_insert(*offset);
            }
        }
        out
    }
}

impl fmt::Display for PointerValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_bottom() {
            return write!(f, "Bot");
        }
        if self.is_top() {
            return write!(f, "Top");
        }
        let mut entries: Vec<_> = self
            .targets
            .iter()
            .map(|(id, offset)| format!("{id}@{offset}"))
            .collect();
        if self.unknown {
            entries.push("UnknownTarget".to_string());
        }
        if self.may_be_null {
            entries.push("Null".to_string());
        }
        if self.may_be_invalid_non_null {
            entries.push("InvalidNonNull".to_string());
        }
        entries.sort();
        write!(f, "{}", entries.join("|"))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LayoutValue {
    pub size: Interval,
    pub align: Interval,
}

impl LayoutValue {
    pub fn bottom() -> Self {
        Self {
            size: Interval::empty(),
            align: Interval::empty(),
        }
    }

    pub fn top() -> Self {
        Self {
            size: Interval::top(),
            align: Interval::top(),
        }
    }

    pub fn new(size: Interval, align: Interval) -> Self {
        Self { size, align }
    }

    pub fn is_bottom(self) -> bool {
        self.size.is_empty() && self.align.is_empty()
    }

    pub fn join(left: Self, right: Self) -> Self {
        if left.is_bottom() {
            return right;
        }
        if right.is_bottom() {
            return left;
        }
        Self {
            size: join_interval(&left.size, &right.size),
            align: join_interval(&left.align, &right.align),
        }
    }

    pub fn widen(previous: Self, next: Self) -> Self {
        if previous.is_bottom() {
            return next;
        }
        if next.is_bottom() {
            return previous;
        }
        Self {
            size: widen(&previous.size, &next.size),
            align: widen(&previous.align, &next.align),
        }
    }
}

impl fmt::Display for LayoutValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "size={} align={}", self.size, self.align)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AbstractBool {
    Bottom,
    False,
    True,
    Top,
}

impl AbstractBool {
    pub fn join(self, other: Self) -> Self {
        match (self, other) {
            (Self::Bottom, value) | (value, Self::Bottom) => value,
            (left, right) if left == right => left,
            _ => Self::Top,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustc_middle::mir::Local;

    fn stack(index: usize) -> AllocationId {
        AllocationId::Stack(Local::from_usize(index))
    }

    #[test]
    fn status_join_preserves_concrete_alternatives() {
        let status = AllocationStatus::live().join(AllocationStatus::dead());
        assert!(status.may_be_live());
        assert!(status.may_be_dead());
        assert!(!status.may_be_absent());
    }

    #[test]
    fn pointer_join_keeps_target_offset_correlation() {
        let left = PointerValue::target(stack(1), Interval::new(0, 0));
        let right = PointerValue::target(stack(2), Interval::new(8, 8));
        let joined = PointerValue::join(&left, &right);
        assert_eq!(joined.target_count(), 2);
        assert_eq!(
            joined
                .targets()
                .find(|(id, _)| **id == stack(1))
                .map(|(_, v)| *v),
            Some(Interval::new(0, 0))
        );
        assert_eq!(
            joined
                .targets()
                .find(|(id, _)| **id == stack(2))
                .map(|(_, v)| *v),
            Some(Interval::new(8, 8))
        );
    }

    #[test]
    fn equality_intersection_refines_offset() {
        let id = stack(1);
        let left = PointerValue::target(id.clone(), Interval::new(0, 8));
        let right = PointerValue::target(id, Interval::new(4, 12));
        let common = PointerValue::intersect_equal(&left, &right);
        assert_eq!(
            common.exact_target().map(|(_, offset)| offset),
            Some(Interval::new(4, 8))
        );
    }

    #[test]
    fn repeated_allocation_loses_singleton_multiplicity() {
        let multiplicity = AllocationMultiplicity::zero()
            .after_allocation()
            .after_allocation();
        assert!(!multiplicity.is_exactly_one());
    }

    #[test]
    fn excluding_null_from_top_keeps_unknown_non_null_targets() {
        let value = PointerValue::top().without_null();
        assert!(value.has_unknown_target());
        assert!(!value.may_be_null());
        assert!(value.is_definitely_non_null());
    }
}
