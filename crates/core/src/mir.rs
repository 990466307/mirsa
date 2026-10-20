use rustc_hir::def_id::DefId;
use rustc_middle::mir::visit::{PlaceContext, Visitor};
use rustc_middle::mir::{Body, Location, Place, PlaceElem, ProjectionElem};
use rustc_middle::ty::TyKind;
use rustc_middle::ty::{Ty, TyCtxt, TypingEnv};
use std::collections::HashSet;

const MAX_PRECOLLECT_ARRAY_ELEMENTS: u64 = 32;

pub fn get_optimized_mir<'tcx>(tcx: TyCtxt<'tcx>, def_id: DefId) -> &'tcx Body<'tcx> {
    tcx.optimized_mir(def_id)
}

struct PlaceCollector<'tcx> {
    places: HashSet<Place<'tcx>>,
}

pub fn for_each_field_projection<'tcx>(
    tcx: TyCtxt<'tcx>,
    ty: Ty<'tcx>,
    mut visit: impl FnMut(&[PlaceElem<'tcx>], Ty<'tcx>),
) {
    match *ty.kind() {
        TyKind::Tuple(_) | TyKind::Closure(_, _) => {
            let fields = match ty.kind() {
                TyKind::Closure(_, args) => args.as_closure().upvar_tys(),
                TyKind::Tuple(fields) => *fields,
                _ => unreachable!(),
            };
            for (idx, field_ty) in fields.iter().enumerate() {
                visit(&[ProjectionElem::Field(idx.into(), field_ty)], field_ty);
            }
        }
        TyKind::Adt(adt_def, substs) => {
            for (variant_index, variant) in adt_def.variants().iter_enumerated() {
                for (idx, field) in variant.fields.iter().enumerate() {
                    let field_ty = tcx
                        .try_normalize_erasing_regions(
                            TypingEnv::fully_monomorphized(),
                            field.ty(tcx, substs),
                        )
                        .unwrap_or_else(|_| field.ty(tcx, substs));
                    let field = ProjectionElem::Field(idx.into(), field_ty);
                    if adt_def.is_enum() {
                        visit(
                            &[ProjectionElem::Downcast(None, variant_index), field],
                            field_ty,
                        );
                    } else {
                        visit(&[field], field_ty);
                    }
                }
            }
        }
        _ => {}
    }
}

fn collect_immediate_projections<'tcx>(
    tcx: TyCtxt<'tcx>,
    places: &mut HashSet<Place<'tcx>>,
    base: Place<'tcx>,
    ty: Ty<'tcx>,
) {
    match ty.kind() {
        TyKind::Ref(_, inner, _) if matches!(inner.kind(), TyKind::Array(..)) => {
            let pointee = base.project_deeper(&[ProjectionElem::Deref], tcx);
            places.insert(pointee);
            collect_immediate_projections(tcx, places, pointee, *inner);
        }
        TyKind::Array(_, len) => {
            if let Some(len) = len.try_to_target_usize(tcx) {
                let len = len as u64;
                if len > MAX_PRECOLLECT_ARRAY_ELEMENTS {
                    return;
                }
                for idx in 0..len {
                    let proj = base.project_deeper(
                        &[ProjectionElem::ConstantIndex {
                            offset: idx,
                            min_length: len,
                            from_end: false,
                        }],
                        tcx,
                    );
                    places.insert(proj);
                }
            }
        }
        TyKind::Tuple(_) | TyKind::Adt(..) => {
            for_each_field_projection(tcx, ty, |projection, field_ty| {
                let field = base.project_deeper(projection, tcx);
                places.insert(field);
                collect_immediate_projections(tcx, places, field, field_ty);
            });
        }
        _ => {}
    }
}

impl<'tcx> Visitor<'tcx> for PlaceCollector<'tcx> {
    fn visit_place(&mut self, place: &Place<'tcx>, context: PlaceContext, location: Location) {
        self.places.insert(*place);
        self.super_place(place, context, location);
    }
}

pub fn collect_body_places<'tcx>(tcx: TyCtxt<'tcx>, body: &Body<'tcx>) -> Vec<Place<'tcx>> {
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

fn is_interval_scalar_ty(ty: rustc_middle::ty::Ty<'_>) -> bool {
    matches!(
        ty.kind(),
        TyKind::Int(_) | TyKind::Uint(_) | TyKind::Bool | TyKind::Char
    )
}

pub fn collect_interval_places<'tcx>(tcx: TyCtxt<'tcx>, body: &Body<'tcx>) -> Vec<Place<'tcx>> {
    collect_body_places(tcx, body)
        .into_iter()
        .filter(|place| is_interval_scalar_ty(place.ty(&body.local_decls, tcx).ty))
        .collect()
}

pub fn collect_ptr_places<'tcx>(tcx: TyCtxt<'tcx>, body: &Body<'tcx>) -> Vec<Place<'tcx>> {
    collect_body_places(tcx, body)
        .into_iter()
        .filter(|place| {
            matches!(
                place.ty(&body.local_decls, tcx).ty.kind(),
                TyKind::RawPtr(_, _) | TyKind::FnPtr(..)
            )
        })
        .collect()
}
