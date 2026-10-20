//! Integer range iterator effects, prepared before call-write invalidation.
use super::abstract_value::{Interval, intersect, join};
use super::state::IntervalState;
use mirsa_framework::access_path::AccessPath;
use mirsa_framework::call_model::RangeIteratorCall;
use mirsa_relations::symbolic::SymbolicState;
use rustc_middle::mir::{LocalDecls, Place, ProjectionElem, Terminator, TerminatorKind};
use rustc_middle::ty::{TyCtxt, TyKind, UintTy};

fn field<'tcx>(
    tcx: TyCtxt<'tcx>,
    decls: &LocalDecls<'tcx>,
    place: Place<'tcx>,
    name: &str,
) -> Option<Place<'tcx>> {
    let TyKind::Adt(def, args) = place.ty(decls, tcx).ty.kind() else {
        return None;
    };
    let (index, field) = def
        .non_enum_variant()
        .fields
        .iter_enumerated()
        .find(|(_, f)| f.name.as_str() == name)?;
    Some(place.project_deeper(&[ProjectionElem::Field(index, field.ty(tcx, args))], tcx))
}

pub struct NextEffects<'tcx> {
    values: Vec<(AccessPath, Place<'tcx>, Interval)>,
    variant: Option<(AccessPath, usize)>,
}
impl<'tcx> NextEffects<'tcx> {
    pub fn apply(self, state: &mut IntervalState<'tcx>) {
        for (path, place, value) in self.values {
            state.set_interval_path(path, place, value);
        }
        if let Some((path, variant)) = self.variant {
            state.set_variant_path(path, variant);
        }
    }
}

pub fn prepare_next<'tcx>(
    tcx: TyCtxt<'tcx>,
    decls: &LocalDecls<'tcx>,
    term: &Terminator<'tcx>,
    symbolic: &SymbolicState<'tcx>,
    state: &IntervalState<'tcx>,
) -> Option<NextEffects<'tcx>> {
    let RangeIteratorCall::Next {
        receiver,
        inclusive,
    } = RangeIteratorCall::classify(tcx, decls, term)?
    else {
        return None;
    };
    let TerminatorKind::Call { destination, .. } = &term.kind else {
        return None;
    };
    let destination_path = symbolic.normalize_place(*destination)?;
    if symbolic.normalize_place(receiver)?.has_index() || destination_path.has_index() {
        return None;
    }
    let start_place = field(tcx, decls, receiver, "start")?;
    let end_place = field(tcx, decls, receiver, "end")?;
    let read = |p| {
        state
            .tracked_interval_resolved(symbolic, &p)
            .unwrap_or_else(Interval::top)
    };
    let start = read(start_place);
    let end = read(end_place);
    // The signed interval carrier cannot bound the upper half of u128.
    // Keep the ordinary conservative fallback if either endpoint may be there.
    if matches!(
        start_place.ty(decls, tcx).ty.kind(),
        TyKind::Uint(UintTy::U128)
    ) && (start.low < 0 || start.high == i128::MAX || end.low < 0 || end.high == i128::MAX)
    {
        return None;
    }
    let exhausted_place = if inclusive {
        Some(field(tcx, decls, receiver, "exhausted")?)
    } else {
        None
    };
    let exhausted = exhausted_place.map(read).unwrap_or(Interval::new(0, 0));
    let step = step(start, end, exhausted, inclusive)?;
    let TyKind::Adt(option, generics) = destination.ty(decls, tcx).ty.kind() else {
        return None;
    };
    if Some(option.did()) != tcx.lang_items().option_type() {
        return None;
    }
    let (some, some_def) = option
        .variants()
        .iter_enumerated()
        .find(|(_, v)| v.name.as_str() == "Some")?;
    let (none, _) = option
        .variants()
        .iter_enumerated()
        .find(|(_, v)| v.name.as_str() == "None")?;
    let mut effects = NextEffects {
        values: Vec::new(),
        variant: step.variant.map(|present| {
            (
                destination_path,
                if present { some.index() } else { none.index() },
            )
        }),
    };
    let mut put = |place, value| -> Option<()> {
        effects
            .values
            .push((symbolic.normalize_place(place)?, place, value));
        Some(())
    };
    put(start_place, step.start)?;
    put(end_place, end)?;
    if let Some(place) = exhausted_place {
        put(place, step.exhausted)?;
    }
    // None has no Some payload. Bottom preserves that fact when joining
    // Some and None results; top would invent arbitrary yielded elements.
    let item = destination.project_deeper(
        &[
            ProjectionElem::Downcast(None, some),
            ProjectionElem::Field(0u32.into(), some_def.fields[0u32.into()].ty(tcx, generics)),
        ],
        tcx,
    );
    put(item, step.item)?;
    Some(effects)
}

struct Step {
    start: Interval,
    exhausted: Interval,
    item: Interval,
    variant: Option<bool>,
}

fn step(start: Interval, end: Interval, exhausted: Interval, inclusive: bool) -> Option<Step> {
    if start.is_empty() || end.is_empty() || exhausted.is_empty() {
        return None;
    }
    let active = exhausted.low <= 0;
    let more = active && start.low < end.high;
    let last = active && inclusive && !intersect(&start, &end).is_empty();
    let present = more || last;
    let absent = exhausted.high > 0
        || if inclusive {
            start.high > end.low
        } else {
            start.high >= end.low
        };
    let upper_before_end = if end.high == i128::MAX {
        i128::MAX
    } else {
        end.high.saturating_sub(1)
    };
    let mut item = Interval::empty();
    let mut next = if absent { start } else { Interval::empty() };
    let mut finished = if absent { exhausted } else { Interval::empty() };
    if more {
        let current = intersect(&start, &Interval::new(i128::MIN, upper_before_end));
        item = join(&item, &current);
        next = join(
            &next,
            &Interval::new(
                current.low.saturating_add(1),
                current.high.saturating_add(1),
            ),
        );
        finished = join(&finished, &Interval::new(0, 0));
    }
    if last {
        let current = intersect(&start, &end);
        item = join(&item, &current);
        next = join(&next, &current);
        finished = join(&finished, &Interval::new(1, 1));
    }
    Some(Step {
        start: next,
        exhausted: finished,
        item,
        variant: match (present, absent) {
            (true, false) => Some(true),
            (false, true) => Some(false),
            _ => None,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn range_steps_enclose_native_iterator_results() {
        let contains = |range: Interval, value| range.low <= value && value <= range.high;
        for inclusive in [false, true] {
            for a in -3..=3 {
                for b in a..=3 {
                    for c in -3..=3 {
                        for d in c..=3 {
                            let abstract_step = step(
                                Interval::new(a, b),
                                Interval::new(c, d),
                                Interval::new(0, 0),
                                inclusive,
                            )
                            .unwrap();
                            for start in a..=b {
                                for end in c..=d {
                                    let (item, next) = if inclusive {
                                        let mut range = start..=end;
                                        let item = range.next();
                                        (item, *range.start())
                                    } else {
                                        let mut range = start..end;
                                        (range.next(), range.start)
                                    };
                                    assert!(contains(abstract_step.start, next));
                                    if let Some(item) = item {
                                        assert!(contains(abstract_step.item, item));
                                    }
                                    if let Some(present) = abstract_step.variant {
                                        assert_eq!(present, item.is_some());
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn inclusive_maximum_is_returned_once_without_overflow() {
        let bound = Interval::new(i128::MAX, i128::MAX);
        let first = step(bound, bound, Interval::new(0, 0), true).unwrap();
        assert_eq!(first.item, bound);
        assert_eq!(first.start, bound);
        assert_eq!(first.exhausted, Interval::new(1, 1));
        assert_eq!(first.variant, Some(true));
        let next = step(first.start, bound, first.exhausted, true).unwrap();
        assert!(next.item.is_empty());
        assert_eq!(next.variant, Some(false));
        assert_eq!(
            step(bound, bound, Interval::new(0, 0), false)
                .unwrap()
                .variant,
            Some(false)
        );
    }
}
