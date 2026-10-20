use rustc_middle::mir::{LocalDecls, Operand};
use rustc_middle::ty::{TyCtxt, TyKind, TypingEnv};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PointerOffset {
    pub bytes: u64,
    pub subtract: bool,
    pub wrapping: bool,
}

impl PointerOffset {
    pub fn classify<'tcx>(
        tcx: TyCtxt<'tcx>,
        func: &Operand<'tcx>,
        local_decls: &LocalDecls<'tcx>,
    ) -> Option<Self> {
        let TyKind::FnDef(callee, args) = *func.ty(local_decls, tcx).kind() else {
            return None;
        };
        if callee.is_local() || tcx.crate_name(callee.krate).as_str() != "core" {
            return None;
        }
        let owner = tcx.impl_of_method(callee)?;
        let pointer_impl = match tcx.type_of(owner).instantiate_identity().kind() {
            TyKind::RawPtr(..) => true,
            TyKind::Adt(adt, _) => {
                tcx.is_diagnostic_item(rustc_span::Symbol::intern("NonNull"), adt.did())
            }
            _ => false,
        };
        if tcx.trait_id_of_impl(owner).is_some() || !pointer_impl {
            return None;
        }
        let name = tcx.item_name(callee);
        let (subtract, wrapping, byte) = match name.as_str() {
            "add" | "offset" => (false, false, false),
            "sub" => (true, false, false),
            "wrapping_add" | "wrapping_offset" => (false, true, false),
            "wrapping_sub" => (true, true, false),
            "byte_add" | "byte_offset" => (false, false, true),
            "byte_sub" => (true, false, true),
            "wrapping_byte_add" | "wrapping_byte_offset" => (false, true, true),
            "wrapping_byte_sub" => (true, true, true),
            _ => return None,
        };
        let bytes = if byte {
            1
        } else {
            tcx.layout_of(TypingEnv::fully_monomorphized().as_query_input(args.type_at(0)))
                .ok()?
                .size
                .bytes()
        };
        Some(Self {
            bytes,
            subtract,
            wrapping,
        })
    }
}
