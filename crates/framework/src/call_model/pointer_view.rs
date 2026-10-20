use rustc_middle::mir::{LocalDecls, Operand, ProjectionElem};
use rustc_middle::ty::{Ty, TyCtxt, TyKind, TypingEnv};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ViewLength {
    Preserve,
    Argument(usize),
    One,
    Unknown,
}

impl ViewLength {
    fn cast<'tcx>(tcx: TyCtxt<'tcx>, source: Ty<'tcx>, target: Ty<'tcx>) -> Self {
        if source == target {
            return Self::Preserve;
        }
        let size = |ty| {
            tcx.layout_of(TypingEnv::fully_monomorphized().as_query_input(ty))
                .ok()
                .map(|layout| layout.size.bytes())
        };
        match (size(source), size(target)) {
            (Some(source), Some(target)) if source > 0 && source == target => Self::Preserve,
            _ => Self::Unknown,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PointerView {
    pub length: ViewLength,
    indirect: bool,
}

impl PointerView {
    pub fn classify<'tcx>(
        tcx: TyCtxt<'tcx>,
        func: &Operand<'tcx>,
        local_decls: &LocalDecls<'tcx>,
    ) -> Option<Self> {
        let TyKind::FnDef(callee, generic_args) = *func.ty(local_decls, tcx).kind() else {
            return None;
        };
        if callee.is_local() || tcx.crate_name(callee.krate).as_str() != "core" {
            return None;
        }
        let name = tcx.item_name(callee);
        let name = name.as_str();
        if name == "from"
            && tcx
                .trait_of_item(callee)
                .is_some_and(|id| tcx.is_diagnostic_item(rustc_span::Symbol::intern("From"), id))
            && matches!(generic_args.type_at(0).kind(), TyKind::Adt(adt, _) if tcx.is_diagnostic_item(rustc_span::Symbol::intern("NonNull"), adt.did()))
            && matches!(generic_args.type_at(1).kind(), TyKind::Ref(..))
        {
            return Some(Self {
                length: ViewLength::Preserve,
                indirect: false,
            });
        }
        let mut indirect = false;
        let length = if let Some(owner) = tcx.impl_of_method(callee) {
            if tcx.trait_id_of_impl(owner).is_some() {
                return None;
            }
            match (tcx.type_of(owner).instantiate_identity().kind(), name) {
                (TyKind::RawPtr(..), "cast") => {
                    ViewLength::cast(tcx, generic_args.type_at(0), generic_args.type_at(1))
                }
                (
                    TyKind::RawPtr(..),
                    "cast_const" | "cast_mut" | "as_ref_unchecked" | "as_mut_unchecked",
                ) => ViewLength::Preserve,
                (TyKind::Slice(_) | TyKind::Str, "as_ptr" | "as_mut_ptr") => ViewLength::Preserve,
                (TyKind::Str, "as_bytes" | "as_bytes_mut") => ViewLength::Preserve,
                (TyKind::Array(..), "as_slice" | "as_mut_slice") => ViewLength::Preserve,
                (TyKind::Adt(adt, _), name) => {
                    if tcx.is_diagnostic_item(rustc_span::Symbol::intern("NonNull"), adt.did()) {
                        match name {
                            "new_unchecked" | "from_ref" | "from_mut" | "as_ptr" | "as_mut_ptr" => {
                                ViewLength::Preserve
                            }
                            "as_ref" | "as_mut" => {
                                indirect = true;
                                ViewLength::Preserve
                            }
                            "cast" => ViewLength::cast(
                                tcx,
                                generic_args.type_at(0),
                                generic_args.type_at(1),
                            ),
                            "slice_from_raw_parts" => ViewLength::Argument(1),
                            _ => return None,
                        }
                    } else if tcx.lang_items().maybe_uninit() == Some(adt.did()) {
                        match name {
                            "as_ptr" | "as_mut_ptr" | "assume_init_ref" | "assume_init_mut" => {
                                ViewLength::Unknown
                            }
                            _ => return None,
                        }
                    } else {
                        return None;
                    }
                }
                _ => return None,
            }
        } else {
            let path = tcx.def_path_str(callee);
            let path = path
                .strip_prefix("core::")
                .or_else(|| path.strip_prefix("std::"))?;
            match path {
                "slice::raw::from_raw_parts"
                | "slice::raw::from_raw_parts_mut"
                | "slice::from_raw_parts"
                | "slice::from_raw_parts_mut" => ViewLength::Argument(1),
                "ptr::slice_from_raw_parts" | "ptr::slice_from_raw_parts_mut" => {
                    ViewLength::Argument(1)
                }
                "slice::raw::from_ref"
                | "slice::raw::from_mut"
                | "slice::from_ref"
                | "slice::from_mut" => ViewLength::One,
                "ptr::from_ref" | "ptr::from_mut" => ViewLength::Preserve,
                "str::converts::from_utf8_unchecked"
                | "str::converts::from_utf8_unchecked_mut"
                | "str::from_utf8_unchecked"
                | "str::from_utf8_unchecked_mut" => ViewLength::Preserve,
                _ => return None,
            }
        };
        Some(Self { length, indirect })
    }

    pub fn source<'tcx>(
        self,
        tcx: TyCtxt<'tcx>,
        local_decls: &LocalDecls<'tcx>,
        argument: &Operand<'tcx>,
    ) -> Option<Operand<'tcx>> {
        if !self.indirect {
            return Some(argument.clone());
        }
        let place = argument.place()?;
        matches!(place.ty(local_decls, tcx).ty.kind(), TyKind::Ref(..))
            .then(|| Operand::Copy(place.project_deeper(&[ProjectionElem::Deref], tcx)))
    }
}
