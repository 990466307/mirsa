use rustc_hir::def_id::DefId;
use rustc_middle::mir::{Body, Terminator, TerminatorKind};
use rustc_middle::ty::{TyCtxt, TyKind};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ContractCall {
    NonZeroNewUnchecked,
    NonNullNewUnchecked,
    CStrFromPtr,
    SliceFromRawParts,
    SliceFromRawPartsMut,
    VecFromRawParts,
    PtrRead,
    PtrWrite,
    PtrCopyNonoverlapping,
    SliceGetUnchecked,
    SliceGetUncheckedMut,
    SliceSplitAtUnchecked,
    SliceSplitAtMutUnchecked,
}

impl ContractCall {
    pub(crate) fn has_internval_contract(self) -> bool {
        matches!(
            self,
            ContractCall::NonZeroNewUnchecked
                | ContractCall::SliceGetUnchecked
                | ContractCall::SliceGetUncheckedMut
                | ContractCall::SliceSplitAtUnchecked
                | ContractCall::SliceSplitAtMutUnchecked
                | ContractCall::PtrCopyNonoverlapping
        )
    }

    pub(crate) fn has_nullptr_contract(self) -> bool {
        matches!(
            self,
            ContractCall::NonNullNewUnchecked
                | ContractCall::CStrFromPtr
                | ContractCall::SliceFromRawParts
                | ContractCall::SliceFromRawPartsMut
                | ContractCall::VecFromRawParts
                | ContractCall::PtrRead
                | ContractCall::PtrWrite
                | ContractCall::PtrCopyNonoverlapping
        )
    }
}

pub(crate) fn call_path<'tcx>(
    tcx: TyCtxt<'tcx>,
    body: &Body<'tcx>,
    term: &Terminator<'tcx>,
) -> Option<String> {
    let def_id = call_def_id(tcx, body, term)?;
    Some(tcx.def_path_str(def_id))
}

fn call_def_id<'tcx>(
    tcx: TyCtxt<'tcx>,
    body: &Body<'tcx>,
    term: &Terminator<'tcx>,
) -> Option<DefId> {
    let TerminatorKind::Call { func, .. } = &term.kind else {
        return None;
    };
    let TyKind::FnDef(def_id, _) = func.ty(&body.local_decls, tcx).kind() else {
        return None;
    };
    Some(*def_id)
}

pub(crate) fn call_path_segments<'tcx>(
    tcx: TyCtxt<'tcx>,
    body: &Body<'tcx>,
    term: &Terminator<'tcx>,
) -> Option<Vec<String>> {
    let def_id = call_def_id(tcx, body, term)?;
    Some(def_path_segments(tcx, def_id))
}

pub(crate) fn classify_call<'tcx>(
    tcx: TyCtxt<'tcx>,
    body: &Body<'tcx>,
    term: &Terminator<'tcx>,
) -> Option<ContractCall> {
    let path = call_path_segments(tcx, body, term)?;
    classify_path(&path)
}

fn def_path_segments(tcx: TyCtxt<'_>, def_id: DefId) -> Vec<String> {
    if let Some(impl_def_id) = tcx.impl_of_method(def_id) {
        let mut segments = match tcx.type_of(impl_def_id).instantiate_identity().kind() {
            TyKind::Adt(def, _) => item_def_path_segments(tcx, def.did()),
            _ => item_def_path_segments(tcx, impl_def_id),
        };
        if let Some(name) = def_path_item_name(tcx, def_id) {
            segments.push(name);
        }
        return segments;
    }

    if let Some(trait_def_id) = tcx.trait_of_item(def_id) {
        let mut segments = item_def_path_segments(tcx, trait_def_id);
        if let Some(name) = def_path_item_name(tcx, def_id) {
            segments.push(name);
        }
        return segments;
    }

    item_def_path_segments(tcx, def_id)
}

fn item_def_path_segments(tcx: TyCtxt<'_>, def_id: DefId) -> Vec<String> {
    let def_path = tcx.def_path(def_id);
    let mut segments = vec![tcx.crate_name(def_path.krate).as_str().to_string()];
    segments.extend(
        def_path
            .data
            .iter()
            .filter_map(|component| component.data.get_opt_name())
            .map(|symbol| symbol.as_str().to_string()),
    );
    segments
}

fn def_path_item_name(tcx: TyCtxt<'_>, def_id: DefId) -> Option<String> {
    tcx.def_key(def_id)
        .disambiguated_data
        .data
        .get_opt_name()
        .map(|symbol| symbol.as_str().to_string())
}

fn classify_path(path: &[String]) -> Option<ContractCall> {
    let path = path.iter().map(String::as_str).collect::<Vec<_>>();

    match path.as_slice() {
        [.., "copy_nonoverlapping"] => Some(ContractCall::PtrCopyNonoverlapping),
        [.., ty, "new_unchecked"] if ty.starts_with("NonZero") => {
            Some(ContractCall::NonZeroNewUnchecked)
        }
        [.., "NonNull", "new_unchecked"] => Some(ContractCall::NonNullNewUnchecked),
        [.., "CStr", "from_ptr"] => Some(ContractCall::CStrFromPtr),
        [.., "from_raw_parts_mut"] if path.contains(&"slice") => {
            Some(ContractCall::SliceFromRawPartsMut)
        }
        [.., "Vec", "from_raw_parts"] | [.., "vec", "from_raw_parts"] => {
            Some(ContractCall::VecFromRawParts)
        }
        [.., "from_raw_parts"] if path.contains(&"slice") => Some(ContractCall::SliceFromRawParts),
        [.., "read"] if path.contains(&"ptr") => Some(ContractCall::PtrRead),
        [.., "write"] if path.contains(&"ptr") => Some(ContractCall::PtrWrite),
        [.., "get_unchecked_mut"] => Some(ContractCall::SliceGetUncheckedMut),
        [.., "get_unchecked"] => Some(ContractCall::SliceGetUnchecked),
        [.., "split_at_mut_unchecked"] => Some(ContractCall::SliceSplitAtMutUnchecked),
        [.., "split_at_unchecked"] => Some(ContractCall::SliceSplitAtUnchecked),
        _ => None,
    }
}
