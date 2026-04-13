use rustc_hir::def_id::DefId;
use rustc_middle::mir::visit::{PlaceContext, Visitor};
use rustc_middle::mir::{Body, Location, Place, ProjectionElem};
use rustc_middle::ty::TyCtxt;
use rustc_middle::ty::TyKind;
use std::collections::HashSet;

pub fn get_optimized_mir<'tcx>(tcx: TyCtxt<'tcx>, def_id: DefId) -> &'tcx Body<'tcx> {
    tcx.optimized_mir(def_id)
}

struct PlaceCollector<'tcx> {
    places: HashSet<Place<'tcx>>,
}

fn collect_immediate_projections<'tcx>(
    tcx: TyCtxt<'tcx>,
    places: &mut HashSet<Place<'tcx>>,
    base: Place<'tcx>,
    ty: rustc_middle::ty::Ty<'tcx>,
) {
    match ty.kind() {
        TyKind::Tuple(fields) => {
            for (idx, field_ty) in fields.iter().enumerate() {
                let proj = base.project_deeper(&[ProjectionElem::Field(idx.into(), field_ty)], tcx);
                places.insert(proj);
                collect_immediate_projections(tcx, places, proj, field_ty);
            }
        }
        TyKind::Adt(adt_def, substs) => {
            for variant in adt_def.variants().iter() {
                for (idx, field) in variant.fields.iter().enumerate() {
                    let field_ty = field.ty(tcx, substs);
                    let proj =
                        base.project_deeper(&[ProjectionElem::Field(idx.into(), field_ty)], tcx);
                    places.insert(proj);
                    collect_immediate_projections(tcx, places, proj, field_ty);
                }
            }
        }
        TyKind::Array(_, len) => {
            if let Some(len) = len.try_to_target_usize(tcx) {
                for idx in 0..len {
                    let proj = base.project_deeper(
                        &[ProjectionElem::ConstantIndex {
                            offset: idx as u64,
                            min_length: len as u64,
                            from_end: false,
                        }],
                        tcx,
                    );
                    places.insert(proj);
                }
            }
        }
        _ => {}
    }
}

impl<'tcx> Visitor<'tcx> for PlaceCollector<'tcx> {
    fn visit_place(&mut self, place: &Place<'tcx>, context: PlaceContext, location: Location) {
        let has_runtime_index = place
            .projection
            .iter()
            .any(|elem| matches!(elem, ProjectionElem::Index(_)));
        if !has_runtime_index {
            self.places.insert(*place);
        }
        self.super_place(place, context, location);
    }
}

pub fn collect_body_places<'tcx>(tcx: TyCtxt<'tcx>, body: &'tcx Body<'tcx>) -> Vec<Place<'tcx>> {
    let mut places: HashSet<Place<'tcx>> = HashSet::new();

    for (local, _decl) in body.local_decls.iter_enumerated() {
        let base = Place::from(local);
        places.insert(base);
        collect_immediate_projections(tcx, &mut places, base, _decl.ty);
    }

    let mut collector = PlaceCollector { places };
    collector.visit_body(body);

    collector.places.into_iter().collect()
}

pub fn collect_ref_places<'tcx>(tcx: TyCtxt<'tcx>, body: &'tcx Body<'tcx>) -> Vec<Place<'tcx>> {
    let _ = tcx;
    body.local_decls
        .iter_enumerated()
        .filter_map(|(local, decl)| {
            if matches!(decl.ty.kind(), TyKind::Ref(_, _, _)) {
                Some(Place::from(local))
            } else {
                None
            }
        })
        .collect()
}

pub fn collect_ptr_places<'tcx>(tcx: TyCtxt<'tcx>, body: &'tcx Body<'tcx>) -> Vec<Place<'tcx>> {
    collect_body_places(tcx, body)
        .into_iter()
        .filter(|place| matches!(place.ty(&body.local_decls, tcx).ty.kind(), TyKind::RawPtr(_, _) | TyKind::FnPtr(..)))
        .collect()
}
