use crate::framework::eq_domain::{EqDomain, join_eq};
use crate::framework::forward::DomainState;
use crate::framework::printer::StateEntries;
use rustc_middle::mir::Place;
use rustc_middle::ty::{Ty, TyCtxt, TyKind};
use std::collections::HashMap;
use std::fmt;

use super::abstract_value::{Internval, join, widen};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InternvalState<'tcx> {
    pub internval: HashMap<Place<'tcx>, Internval>,
    pub slice_meta: HashMap<Place<'tcx>, Internval>,
    pub eq: EqDomain<'tcx>,
}

impl<'tcx> InternvalState<'tcx> {
    fn default() -> Self {
        InternvalState {
            internval: HashMap::new(),
            slice_meta: HashMap::new(),
            eq: EqDomain::new(),
        }
    }

    pub fn new_bot_state(places: &[Place<'tcx>], arg_count: usize) -> Self {
        let mut internval = HashMap::new();
        let mut eq = EqDomain::new();

        for place in places {
            let local_idx = place.local.index();
            let value = if local_idx >= 1 && local_idx <= arg_count {
                Internval::top()
            } else {
                Internval::empty()
            };
            internval.insert(*place, value);
            eq.kill(*place);
        }

        InternvalState {
            internval,
            slice_meta: HashMap::new(),
            eq,
        }
    }
}

pub(crate) fn scalar_layout<'tcx>(tcx: TyCtxt<'tcx>, ty: Ty<'tcx>) -> Option<(u64, bool)> {
    match ty.kind() {
        TyKind::Int(int_ty) => Some((
            int_ty
                .bit_width()
                .unwrap_or_else(|| tcx.data_layout.pointer_size.bits()),
            true,
        )),
        TyKind::Uint(uint_ty) => Some((
            uint_ty
                .bit_width()
                .unwrap_or_else(|| tcx.data_layout.pointer_size.bits()),
            false,
        )),
        TyKind::Bool => Some((1, false)),
        TyKind::Char => Some((32, false)),
        _ => None,
    }
}

pub(crate) fn unsigned_bits_to_i128(bits: u128, bit_width: u64) -> i128 {
    if bit_width == 128 {
        if bits <= i128::MAX as u128 {
            bits as i128
        } else {
            i128::MAX
        }
    } else {
        let mask = (1u128 << bit_width) - 1;
        (bits & mask) as i128
    }
}

pub(crate) fn signed_bits_to_i128(bits: u128, bit_width: u64) -> i128 {
    if bit_width == 128 {
        return bits as i128;
    }

    let sign_bit = 1u128 << (bit_width - 1);
    let mask = (1u128 << bit_width) - 1;
    let x = bits & mask;

    if (x & sign_bit) != 0 {
        (x as i128) - ((1u128 << bit_width) as i128)
    } else {
        x as i128
    }
}

pub(crate) fn bits_to_i128(bits: u128, bit_width: u64, signed: bool) -> i128 {
    if signed {
        signed_bits_to_i128(bits, bit_width)
    } else {
        unsigned_bits_to_i128(bits, bit_width)
    }
}

pub(crate) fn switch_value_to_i128<'tcx>(
    tcx: TyCtxt<'tcx>,
    ty: Ty<'tcx>,
    value: u128,
) -> Option<i128> {
    let (bit_width, signed) = scalar_layout(tcx, ty)?;
    Some(bits_to_i128(value, bit_width, signed))
}

impl<'tcx> DomainState<'tcx> for InternvalState<'tcx> {
    fn join(left: &Self, right: &Self) -> Self {
        join_state(left, right)
    }

    fn widen(previous: &Self, next: &Self) -> Self {
        widen_state(previous, next)
    }

    fn state_changed(previous: &Self, next: &Self) -> bool {
        previous.internval != next.internval || previous.slice_meta != next.slice_meta
    }
}

impl<'tcx> fmt::Display for InternvalState<'tcx> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut entries: Vec<(String, String)> = self
            .internval
            .iter()
            .map(|(place, interval)| (format!("{place:?}"), interval.to_string()))
            .collect();
        entries.sort_by(|a, b| a.0.cmp(&b.0));

        for (idx, (place, interval)) in entries.iter().enumerate() {
            if idx > 0 {
                write!(f, ", ")?;
            }
            write!(f, "{place} => {interval}")?;
        }
        Ok(())
    }
}

impl<'tcx> StateEntries<'tcx> for InternvalState<'tcx> {
    fn entries(&self) -> Vec<(Place<'tcx>, String)> {
        self.internval
            .iter()
            .map(|(place, interval)| (*place, interval.to_string()))
            .collect()
    }

    fn should_print_entry(&self, place: Place<'tcx>) -> bool {
        self.internval
            .get(&place)
            .is_some_and(|interval| !interval.is_empty())
    }
}

impl<'tcx> InternvalState<'tcx> {
    pub fn get_internval(&self, place: &Place<'tcx>) -> Internval {
        self.internval
            .get(place)
            .copied()
            .unwrap_or_else(Internval::empty)
    }
    pub fn set_internval(&mut self, place: Place<'tcx>, internval: Internval) {
        self.internval.insert(place, internval);
    }

    pub fn get_slice_meta(&self, place: &Place<'tcx>) -> Option<Internval> {
        self.slice_meta.get(place).copied()
    }

    pub fn set_slice_meta(&mut self, place: Place<'tcx>, internval: Internval) {
        self.slice_meta.insert(place, internval);
    }

    pub fn clear_slice_meta(&mut self, place: &Place<'tcx>) {
        self.slice_meta.remove(place);
    }
}

pub fn join_state<'tcx>(
    a: &InternvalState<'tcx>,
    b: &InternvalState<'tcx>,
) -> InternvalState<'tcx> {
    let mut out = InternvalState::default();
    for k in a.internval.keys().chain(b.internval.keys()) {
        let ia = a.internval.get(k).copied().unwrap_or_else(Internval::empty);
        let ib = b.internval.get(k).copied().unwrap_or_else(Internval::empty);
        out.internval.insert(*k, join(&ia, &ib));
    }
    for k in a.slice_meta.keys().chain(b.slice_meta.keys()) {
        let ia = a
            .slice_meta
            .get(k)
            .copied()
            .unwrap_or_else(Internval::empty);
        let ib = b
            .slice_meta
            .get(k)
            .copied()
            .unwrap_or_else(Internval::empty);
        out.slice_meta.insert(*k, join(&ia, &ib));
    }
    out.eq = join_eq(&a.eq, &b.eq);
    out
}

pub fn widen_state<'tcx>(
    a: &InternvalState<'tcx>,
    b: &InternvalState<'tcx>,
) -> InternvalState<'tcx> {
    let mut out = InternvalState::default();
    for k in a.internval.keys().chain(b.internval.keys()) {
        let ia = a.internval.get(k).copied().unwrap_or_else(Internval::empty);
        let ib = b.internval.get(k).copied().unwrap_or_else(Internval::empty);
        let widened = widen(&ia, &ib);
        out.internval.insert(*k, widened);
    }
    for k in a.slice_meta.keys().chain(b.slice_meta.keys()) {
        let ia = a
            .slice_meta
            .get(k)
            .copied()
            .unwrap_or_else(Internval::empty);
        let ib = b
            .slice_meta
            .get(k)
            .copied()
            .unwrap_or_else(Internval::empty);
        out.slice_meta.insert(*k, widen(&ia, &ib));
    }
    out.eq = join_eq(&a.eq, &b.eq);
    out
}
