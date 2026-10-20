mod pointer_offset;
pub use pointer_offset::PointerOffset;
mod pointer_view;
pub use pointer_view::{PointerView, ViewLength};
mod range_iterator;
pub use range_iterator::RangeIteratorCall;

use rustc_middle::mir::{LocalDecls, Operand};
use rustc_middle::ty::{Ty, TyCtxt, TyKind};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BufferCall {
    Drop,
    Forget,
    AllocateBox,
    BoxIntoVec,
    EmptyVec,
    AllocateVec { count_argument: usize },
    VecFromRawParts,
    VecData,
    VecMetadata,
    VecIntoRawParts,
    Unknown,
}

impl BufferCall {
    pub fn classify<'tcx>(
        tcx: TyCtxt<'tcx>,
        func: &Operand<'tcx>,
        local_decls: &LocalDecls<'tcx>,
    ) -> Self {
        let TyKind::FnDef(def_id, _) = func.ty(local_decls, tcx).kind() else {
            return Self::Unknown;
        };
        let path = tcx.def_path_str(*def_id);
        match path.as_str() {
            "core::mem::drop" | "std::mem::drop" => return Self::Drop,
            "core::mem::forget" | "std::mem::forget" => return Self::Forget,
            "alloc::alloc::exchange_malloc" => return Self::AllocateBox,
            _ => {}
        }
        let Some(module) = path
            .strip_prefix("alloc::")
            .or_else(|| path.strip_prefix("std::"))
        else {
            return Self::Unknown;
        };
        let name = module.rsplit("::").next().unwrap_or_default();
        if module.starts_with("slice::") && name == "into_vec" {
            return Self::BoxIntoVec;
        }
        if !module.starts_with("vec::") {
            return Self::Unknown;
        }
        match name {
            "new" | "new_in" => Self::EmptyVec,
            "with_capacity" | "with_capacity_in" => Self::AllocateVec { count_argument: 0 },
            "from_elem" | "from_elem_in" => Self::AllocateVec { count_argument: 1 },
            "from_raw_parts" | "from_raw_parts_in" => Self::VecFromRawParts,
            "as_ptr" | "as_mut_ptr" => Self::VecData,
            "len" | "capacity" => Self::VecMetadata,
            "into_raw_parts" => Self::VecIntoRawParts,
            _ => Self::Unknown,
        }
    }

    pub fn preserves_memory(self) -> bool {
        matches!(self, Self::VecData | Self::VecMetadata)
    }
}

pub fn is_vec_ty(tcx: TyCtxt<'_>, ty: Ty<'_>) -> bool {
    matches!(ty.kind(), TyKind::Adt(def, _) if matches!(tcx.def_path_str(def.did()).as_str(), "alloc::vec::Vec" | "std::vec::Vec"))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ValueQuery {
    Length,
    Address,
    IsNull,
    IsEmpty,
}

impl ValueQuery {
    pub fn classify(tcx: TyCtxt<'_>, callee: rustc_hir::def_id::DefId) -> Option<Self> {
        let impl_id = tcx.impl_of_method(callee)?;
        if tcx.trait_id_of_impl(impl_id).is_some() {
            return None;
        }
        let ty = tcx.type_of(impl_id).instantiate_identity();
        let name = tcx.item_name(callee);
        match (ty.kind(), name.as_str()) {
            (TyKind::Slice(_) | TyKind::Str, "len") => Some(Self::Length),
            (TyKind::Slice(_) | TyKind::Str, "is_empty") => Some(Self::IsEmpty),
            (TyKind::RawPtr(_, _), "addr" | "expose_provenance") => Some(Self::Address),
            (TyKind::RawPtr(_, _), "is_null") => Some(Self::IsNull),
            (TyKind::RawPtr(inner, _), "len") if matches!(inner.kind(), TyKind::Slice(_)) => {
                Some(Self::Length)
            }
            _ => None,
        }
    }
}
