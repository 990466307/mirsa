use rustc_middle::mir::{LocalDecls, Operand, Place, ProjectionElem, Terminator, TerminatorKind};
use rustc_middle::ty::{Ty, TyCtxt, TyKind};
use rustc_span::Symbol;

/// Standard integer range calls neither retain their receiver nor access
/// unrelated escaped memory. Next mutates only the range itself.
pub enum RangeIteratorCall<'tcx> {
    Identity(Operand<'tcx>),
    Next {
        receiver: Place<'tcx>,
        inclusive: bool,
    },
}
impl<'tcx> RangeIteratorCall<'tcx> {
    pub fn classify(
        tcx: TyCtxt<'tcx>,
        decls: &LocalDecls<'tcx>,
        term: &Terminator<'tcx>,
    ) -> Option<Self> {
        let TerminatorKind::Call {
            func,
            args,
            destination,
            ..
        } = &term.kind
        else {
            return None;
        };
        let TyKind::FnDef(callee, _) = func.ty(decls, tcx).kind() else {
            return None;
        };
        if args.len() != 1 {
            return None;
        }
        if method(tcx, *callee, "into_iter", "IntoIterator") {
            let ty = args[0].node.ty(decls, tcx);
            if ty != destination.ty(decls, tcx).ty {
                return None;
            }
            let mut inner = ty;
            while let TyKind::Ref(_, pointee, _) = inner.kind() {
                inner = *pointee;
            }
            range_type(tcx, inner)?;
            return Some(Self::Identity(args[0].node.clone()));
        }
        if !method(tcx, *callee, "next", "Iterator") {
            return None;
        }
        let mut receiver = args[0].node.place()?;
        if !matches!(receiver.ty(decls, tcx).ty.kind(), TyKind::Ref(..)) {
            return None;
        }
        while matches!(receiver.ty(decls, tcx).ty.kind(), TyKind::Ref(..)) {
            receiver = receiver.project_deeper(&[ProjectionElem::Deref], tcx);
        }
        Some(Self::Next {
            receiver,
            inclusive: range_type(tcx, receiver.ty(decls, tcx).ty)?,
        })
    }
}

fn method(tcx: TyCtxt<'_>, callee: rustc_hir::def_id::DefId, name: &str, trait_name: &str) -> bool {
    let owner = tcx.trait_of_item(callee).or_else(|| {
        tcx.impl_of_method(callee)
            .and_then(|id| tcx.trait_id_of_impl(id))
    });
    tcx.item_name(callee).as_str() == name
        && owner.is_some()
        && owner == tcx.get_diagnostic_item(Symbol::intern(trait_name))
}

fn range_type(tcx: TyCtxt<'_>, ty: Ty<'_>) -> Option<bool> {
    let TyKind::Adt(def, args) = ty.kind() else {
        return None;
    };
    if def.did().is_local() || tcx.crate_name(def.did().krate).as_str() != "core" {
        return None;
    }
    let inclusive = match tcx.item_name(def.did()).as_str() {
        "Range" => false,
        "RangeInclusive" => true,
        _ => return None,
    };
    matches!(args.type_at(0).kind(), TyKind::Int(_) | TyKind::Uint(_)).then_some(inclusive)
}
