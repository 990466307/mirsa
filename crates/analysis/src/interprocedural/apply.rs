use super::context::{CallContext, PlaceMap};
use super::summary::{ExitEffects, project_state};
use crate::reduced_product::CombinedState;
use crate::reduced_product::engine::invalidate_memory;
use mirsa_domains::allocation::abstract_value::{AllocationFact, AllocationId, AllocationSite};
use mirsa_framework::access_path::AccessPath;
use mirsa_framework::mapping::PathMapping;
use rustc_hir::def_id::DefId;
use rustc_middle::mir::{Body, Local, Location, Place, RETURN_PLACE};
use rustc_middle::ty::TyCtxt;

struct OutputMap<'a, 'tcx> {
    map: PlaceMap<'a, 'tcx>,
    destination: AccessPath,
}

impl<'tcx> PathMapping<'tcx> for OutputMap<'_, 'tcx> {
    fn path(&self, path: &AccessPath) -> Option<AccessPath> {
        let mapped = if path.root == RETURN_PLACE {
            self.destination.join_suffix(&path.elems)
        } else {
            self.map.path(path)?
        };
        if path.root != RETURN_PLACE
            && (mapped.strip_pattern_prefix(&self.destination).is_some()
                || self.destination.strip_pattern_prefix(&mapped).is_some())
        {
            return None;
        }
        Some(mapped)
    }

    fn place(&self, place: Place<'tcx>) -> Option<Place<'tcx>> {
        self.path(&AccessPath::from_place(place)?)?;
        self.map.place(place)
    }
}

pub(super) fn apply<'tcx>(
    tcx: TyCtxt<'tcx>,
    body: &Body<'tcx>,
    location: Location,
    callee: DefId,
    caller: &CombinedState<'tcx>,
    context: &CallContext<'tcx>,
    destination: Place<'tcx>,
    effects: &ExitEffects<'tcx>,
) -> Option<CombinedState<'tcx>> {
    let ExitEffects::State(state) = effects else {
        return None;
    };
    let destination_site = AccessPath::from_place(destination)?;
    let locals = context
        .locals
        .iter()
        .enumerate()
        .map(|(index, local)| {
            (
                Local::from_usize(context.first_input + index),
                Place::from(*local),
            )
        })
        .collect::<std::collections::HashMap<_, _>>();
    let mut base = PlaceMap {
        tcx,
        locals,
        symbolic: None,
    };
    let written: Vec<_> = state
        .writes
        .iter()
        .flat_map(|writes| writes.iter())
        .filter_map(|path| base.path(path))
        .collect();
    let object_map = |id: &AllocationId| {
        if matches!(id, AllocationId::Static(_)) {
            return Some(id.clone());
        }
        Some(context.objects.get(id).cloned().unwrap_or_else(|| {
            AllocationId::Call {
                site: AllocationSite {
                    span: body.basic_blocks[location.block]
                        .terminator()
                        .source_info
                        .span,
                    destination: destination_site.clone(),
                },
                caller: body.source.def_id(),
                location,
                callee,
                object: Box::new(id.clone()),
            }
        }))
    };
    let mut out = caller.clone();
    invalidate_memory(&mut out, &written);
    out.extend(project_state(state, &base, &object_map));
    let destination_path = out.symbolic.normalize_place(destination)?;
    base.locals.insert(RETURN_PLACE, destination);
    let map = OutputMap {
        map: base,
        destination: destination_path.clone(),
    };
    let mut projected = project_state(state, &map, &object_map);
    let new_objects: Vec<_> = state
        .allocation
        .objects()
        .filter(|(id, _)| {
            !context.objects.contains_key(*id) && !matches!(id, AllocationId::Static(_))
        })
        .filter_map(|(id, fact)| Some((object_map(id)?, fact.clone())))
        .collect();
    for (id, mut fact) in new_objects {
        if let Some(previous) = caller.allocation.object(&id) {
            fact = AllocationFact::join(previous, &fact);
            fact.multiplicity = previous
                .multiplicity
                .after_allocation()
                .join(fact.multiplicity);
        }
        projected.allocation.set_object(id, fact);
    }
    invalidate_memory(&mut out, &[destination_path]);
    out.extend(projected);
    Some(out)
}
