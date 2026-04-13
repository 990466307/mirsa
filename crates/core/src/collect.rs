use rustc_middle::ty::TyCtxt;

pub fn collect_local_fns<'tcx>(tcx: TyCtxt<'tcx>) -> Vec<rustc_hir::def_id::DefId> {
    let mut result = Vec::new();
    for local_id in tcx.iter_local_def_id() {
        if tcx.def_kind(local_id).is_fn_like() {
            result.push(local_id.to_def_id());
        }
    }
    result
}
