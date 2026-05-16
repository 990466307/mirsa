use rustc_middle::mir::{
    Body, Local, Operand, Place, Rvalue, StatementKind, Terminator, TerminatorKind,
};
use rustc_middle::ty::{Ty, TyCtxt, TyKind};
use rustc_span::Span;

use crate::internval::abstract_value::sub;
use crate::internval::{Internval, InternvalState};

use super::super::transfer::eval_operand;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CheckLevel {
    Safe,
    Possible,
    Definite,
}

impl CheckLevel {
    pub(crate) fn combine(self, other: CheckLevel) -> CheckLevel {
        if self == CheckLevel::Definite || other == CheckLevel::Definite {
            CheckLevel::Definite
        } else if self == CheckLevel::Possible || other == CheckLevel::Possible {
            CheckLevel::Possible
        } else {
            CheckLevel::Safe
        }
    }

    pub(crate) fn oob_code(self) -> &'static str {
        match self {
            CheckLevel::Definite => "internval/definite-oob",
            CheckLevel::Possible => "internval/possible-oob",
            CheckLevel::Safe => unreachable!(),
        }
    }
}

pub(crate) fn call_path<'tcx>(
    tcx: TyCtxt<'tcx>,
    body: &Body<'tcx>,
    term: &Terminator<'tcx>,
) -> Option<String> {
    let TerminatorKind::Call { func, .. } = &term.kind else {
        return None;
    };
    let TyKind::FnDef(def_id, _) = func.ty(&body.local_decls, tcx).kind() else {
        return None;
    };
    Some(tcx.def_path_str(*def_id))
}

pub(crate) fn format_callsite<'tcx>(tcx: TyCtxt<'tcx>, span: Span) -> (String, String) {
    let sm = tcx.sess.source_map();
    let loc = sm.span_to_diagnostic_string(span);
    let snippet = sm
        .span_to_snippet(span)
        .unwrap_or_else(|_| "unsafe call".to_string())
        .replace('\n', " ");
    (loc, snippet)
}

pub(crate) fn emit_warning<'tcx>(
    tcx: TyCtxt<'tcx>,
    span: Span,
    code: &str,
    message: &str,
    notes: &[String],
) {
    let (loc, snippet) = format_callsite(tcx, span);
    println!("warning[{code}]: {message}");
    println!("  --> {loc}");
    println!("   |");
    println!("   | {snippet}");
    println!("   | ^ unsafe call here");
    for note in notes {
        println!("   = {note}");
    }
}

pub(crate) fn eval_call_arg<'tcx>(
    tcx: TyCtxt<'tcx>,
    body: &Body<'tcx>,
    state: &InternvalState<'tcx>,
    arg: &Operand<'tcx>,
) -> Internval {
    eval_operand(tcx, &body.local_decls, arg, state)
}

pub(crate) fn operand_len<'tcx>(
    tcx: TyCtxt<'tcx>,
    body: &Body<'tcx>,
    state: &InternvalState<'tcx>,
    op: &Operand<'tcx>,
) -> Internval {
    match op {
        Operand::Copy(place) | Operand::Move(place) => place_len(tcx, body, state, *place),
        Operand::Constant(_) => Internval::top(),
    }
}

pub(crate) fn place_len<'tcx>(
    tcx: TyCtxt<'tcx>,
    body: &Body<'tcx>,
    state: &InternvalState<'tcx>,
    place: Place<'tcx>,
) -> Internval {
    let ty = place.ty(&body.local_decls, tcx).ty;
    match ty.kind() {
        TyKind::Slice(_) => state.get_slice_meta(&place).unwrap_or_else(Internval::top),
        TyKind::Ref(_, inner, _) if matches!(inner.kind(), TyKind::Slice(_)) => {
            state.get_slice_meta(&place).unwrap_or_else(Internval::top)
        }
        _ => static_len(tcx, ty).unwrap_or_else(Internval::top),
    }
}

fn static_len<'tcx>(tcx: TyCtxt<'tcx>, ty: Ty<'tcx>) -> Option<Internval> {
    match ty.kind() {
        TyKind::Array(_, len) => len
            .try_to_target_usize(tcx)
            .map(|len| Internval::new(len as i128, len as i128)),
        TyKind::Ref(_, inner, _) => static_len(tcx, *inner),
        _ => None,
    }
}

fn raw_ptr_pointee_len<'tcx>(tcx: TyCtxt<'tcx>, ty: Ty<'tcx>) -> Option<Internval> {
    let TyKind::RawPtr(inner, _) = ty.kind() else {
        return None;
    };
    static_len(tcx, *inner)
}

fn place_object_len<'tcx>(
    tcx: TyCtxt<'tcx>,
    body: &Body<'tcx>,
    state: &InternvalState<'tcx>,
    place: Place<'tcx>,
) -> Option<Internval> {
    let ty = place.ty(&body.local_decls, tcx).ty;
    match ty.kind() {
        TyKind::Slice(_) => state.get_slice_meta(&place),
        TyKind::Ref(_, inner, _) if matches!(inner.kind(), TyKind::Slice(_)) => {
            state.get_slice_meta(&place)
        }
        _ => static_len(tcx, ty),
    }
}

fn operand_object_len<'tcx>(
    tcx: TyCtxt<'tcx>,
    body: &Body<'tcx>,
    state: &InternvalState<'tcx>,
    op: &Operand<'tcx>,
) -> Option<Internval> {
    match op {
        Operand::Copy(place) | Operand::Move(place) => place_object_len(tcx, body, state, *place),
        Operand::Constant(_) => None,
    }
}

fn local_place(place: Place<'_>) -> Option<Local> {
    place.projection.is_empty().then_some(place.local)
}

fn local_from_operand(op: &Operand<'_>) -> Option<Local> {
    match op {
        Operand::Copy(place) | Operand::Move(place) => local_place(*place),
        Operand::Constant(_) => None,
    }
}

fn find_local_assignment<'a, 'tcx>(
    body: &'a Body<'tcx>,
    local: Local,
) -> Option<&'a Rvalue<'tcx>> {
    for bbdata in body.basic_blocks.iter() {
        for stmt in &bbdata.statements {
            let StatementKind::Assign(assign) = &stmt.kind else {
                continue;
            };
            let (place, rvalue) = &**assign;
            if place.local == local && place.projection.is_empty() {
                return Some(rvalue);
            }
        }
    }
    None
}

fn find_local_call<'a, 'tcx>(
    body: &'a Body<'tcx>,
    local: Local,
) -> Option<&'a Terminator<'tcx>> {
    for bbdata in body.basic_blocks.iter() {
        let Some(term) = bbdata.terminator.as_ref() else {
            continue;
        };
        let TerminatorKind::Call { destination, .. } = &term.kind else {
            continue;
        };
        if destination.local == local && destination.projection.is_empty() {
            return Some(term);
        }
    }
    None
}

fn pointer_len_from_local<'tcx>(
    tcx: TyCtxt<'tcx>,
    body: &Body<'tcx>,
    state: &InternvalState<'tcx>,
    local: Local,
    depth: u8,
) -> Option<Internval> {
    if depth == 0 {
        return None;
    }

    let place = Place::from(local);
    if let Some(rvalue) = find_local_assignment(body, local) {
        match rvalue {
            Rvalue::Use(op) | Rvalue::Cast(_, op, _) => {
                if let Some(len) = pointer_len_from_operand(tcx, body, state, op, depth - 1) {
                    return Some(len);
                }
            }
            Rvalue::RawPtr(_, borrowed_place) => {
                if let Some(len) = static_len(tcx, borrowed_place.ty(&body.local_decls, tcx).ty) {
                    return Some(len);
                }
            }
            _ => {}
        }
    }

    if let Some(term) = find_local_call(body, local) {
        let path = call_path(tcx, body, term)?;
        let TerminatorKind::Call {
            args, destination, ..
        } = &term.kind
        else {
            return None;
        };

        if path.ends_with("::as_ptr") || path.ends_with("::as_mut_ptr") {
            return args
                .first()
                .and_then(|arg| operand_object_len(tcx, body, state, &arg.node))
                .or_else(|| raw_ptr_pointee_len(tcx, destination.ty(&body.local_decls, tcx).ty));
        }

        if path.ends_with("::add") && args.len() >= 2 {
            let base = pointer_len_from_operand(tcx, body, state, &args[0].node, depth - 1)?;
            let offset = eval_call_arg(tcx, body, state, &args[1].node);
            return Some(sub(&base, &offset));
        }
    }

    raw_ptr_pointee_len(tcx, place.ty(&body.local_decls, tcx).ty)
}

pub(crate) fn pointer_len_from_operand<'tcx>(
    tcx: TyCtxt<'tcx>,
    body: &Body<'tcx>,
    state: &InternvalState<'tcx>,
    op: &Operand<'tcx>,
    depth: u8,
) -> Option<Internval> {
    let local = local_from_operand(op)?;
    pointer_len_from_local(tcx, body, state, local, depth)
}

pub(crate) fn check_lt(index: Internval, len: Internval) -> CheckLevel {
    if index.is_empty() || len.is_empty() {
        return CheckLevel::Safe;
    }
    if index.high < 0 {
        return CheckLevel::Definite;
    }
    if index.low >= 0 && index.high < len.low {
        return CheckLevel::Safe;
    }
    if index.low >= len.high {
        return CheckLevel::Definite;
    }
    CheckLevel::Possible
}

pub(crate) fn check_le(index: Internval, len: Internval) -> CheckLevel {
    if index.is_empty() || len.is_empty() {
        return CheckLevel::Safe;
    }
    if index.high < 0 {
        return CheckLevel::Definite;
    }
    if index.low >= 0 && index.high <= len.low {
        return CheckLevel::Safe;
    }
    if index.low > len.high {
        return CheckLevel::Definite;
    }
    CheckLevel::Possible
}
