use super::summary::project_state;
use crate::reduced_product::CombinedState;
use mirsa_domains::allocation::abstract_value::{AllocationId, AllocationSite};
use mirsa_framework::access_path::AccessPath;
use mirsa_framework::mapping::PathMapping;
use mirsa_relations::symbolic::SymbolicState;
use rustc_middle::mir::visit::{MutVisitor, PlaceContext, TyContext, Visitor};
use rustc_middle::mir::{Body, Local, LocalDecls, Operand, Place, ProjectionElem};
use rustc_middle::mir::{ConstOperand, Location};
use rustc_middle::ty::{self, Instance, Ty, TyCtxt, TypingEnv};
use std::collections::{HashMap, HashSet};

const MAX_INPUT_LOCALS: usize = 128;
const MAX_INPUT_OBJECTS: usize = 128;

#[derive(Clone, Debug, PartialEq)]
pub(super) struct InputContext<'tcx> {
    pub state: CombinedState<'tcx>,
    pub args: Vec<Option<Operand<'tcx>>>,
    pub types: Vec<Ty<'tcx>>,
}

pub(super) struct CallContext<'tcx> {
    pub input: InputContext<'tcx>,
    pub locals: Vec<Local>,
    pub objects: HashMap<AllocationId, AllocationId>,
    pub first_input: usize,
}

pub(super) struct PlaceMap<'a, 'tcx> {
    pub tcx: TyCtxt<'tcx>,
    pub locals: HashMap<Local, Place<'tcx>>,
    pub symbolic: Option<&'a SymbolicState<'tcx>>,
}

impl<'tcx> PlaceMap<'_, 'tcx> {
    fn direct_place(&self, place: Place<'tcx>) -> Option<Place<'tcx>> {
        let root = *self.locals.get(&place.local)?;
        let projection = place
            .projection
            .iter()
            .map(|elem| match elem {
                ProjectionElem::Index(index) => {
                    let mapped = self.locals.get(&index)?;
                    mapped
                        .projection
                        .is_empty()
                        .then_some(ProjectionElem::Index(mapped.local))
                }
                _ => Some(elem),
            })
            .collect::<Option<Vec<_>>>()?;
        Some(root.project_deeper(&projection, self.tcx))
    }

    pub fn allocation(&self, id: &AllocationId) -> Option<AllocationId> {
        Some(match id {
            AllocationId::Stack(local) => {
                let place = self.locals.get(local)?;
                if !place.projection.is_empty() {
                    return None;
                }
                AllocationId::Stack(place.local)
            }
            AllocationId::External(local) => {
                let place = self.locals.get(local)?;
                if !place.projection.is_empty() {
                    return None;
                }
                AllocationId::External(place.local)
            }
            AllocationId::ExternalPath(path) => AllocationId::ExternalPath(self.path(path)?),
            AllocationId::ExternalBuffer(path) => AllocationId::ExternalBuffer(self.path(path)?),
            AllocationId::Heap(site) => AllocationId::Heap(AllocationSite {
                span: site.span,
                destination: self.path(&site.destination)?,
            }),
            AllocationId::Static(_) | AllocationId::Input { .. } | AllocationId::Call { .. } => {
                id.clone()
            }
        })
    }
}

impl<'tcx> PathMapping<'tcx> for PlaceMap<'_, 'tcx> {
    fn path(&self, path: &AccessPath) -> Option<AccessPath> {
        let root = AccessPath::from_place(*self.locals.get(&path.root)?)?;
        Some(root.join_suffix(&path.elems))
    }

    fn place(&self, place: Place<'tcx>) -> Option<Place<'tcx>> {
        self.direct_place(place).or_else(|| {
            self.symbolic?
                .equivalent_places(place, mirsa_framework::reduction::MAX_RELATED)
                .into_iter()
                .find_map(|place| self.direct_place(place))
        })
    }
}

impl<'tcx> CallContext<'tcx> {
    pub fn new(
        tcx: TyCtxt<'tcx>,
        caller: &CombinedState<'tcx>,
        decls: &LocalDecls<'tcx>,
        callee: &Body<'tcx>,
        args: &[Operand<'tcx>],
    ) -> Option<Self> {
        let mut usage = ParameterUsage {
            args: HashSet::new(),
            arg_count: callee.arg_count,
            external: false,
        };
        Visitor::visit_body(&mut usage, callee);
        let used = |index: usize| usage.args.contains(&Local::from_usize(index + 1));
        let mut roots: HashSet<_> = args
            .iter()
            .enumerate()
            .filter(|(index, _)| used(*index))
            .filter_map(|(_, arg)| arg.place().map(|place| place.local))
            .collect();
        loop {
            let previous = roots.len();
            roots = caller.symbolic.related_locals(&roots, usage.external);
            for id in caller.allocation.referenced_objects(&roots) {
                match id {
                    AllocationId::Stack(local) | AllocationId::External(local) => {
                        roots.insert(local);
                    }
                    AllocationId::ExternalPath(path) | AllocationId::ExternalBuffer(path) => {
                        roots.insert(path.root);
                    }
                    _ => {}
                }
            }
            if roots.len() > MAX_INPUT_LOCALS {
                return None;
            }
            if previous == roots.len() {
                break;
            }
        }
        let mut locals: Vec<_> = roots.iter().copied().collect();
        locals.sort_by_key(|local| local.index());
        let first_input = callee.local_decls.len();
        let map = PlaceMap {
            tcx,
            locals: locals
                .iter()
                .enumerate()
                .map(|(index, local)| (*local, Place::from(Local::from_usize(first_input + index))))
                .collect(),
            symbolic: None,
        };
        let mut referenced = caller.allocation.referenced_objects(&roots);
        for local in &locals {
            referenced.insert(AllocationId::Stack(*local));
        }
        if referenced.len() > MAX_INPUT_OBJECTS {
            return None;
        }
        let mut referenced: Vec<_> = referenced.into_iter().collect();
        referenced.sort_by_cached_key(|id| format!("{id:?}"));
        let mut objects = HashMap::new();
        for (index, original) in referenced.into_iter().enumerate() {
            let mapped = match &original {
                AllocationId::Stack(_) => map.allocation(&original)?,
                AllocationId::Static(_) => original.clone(),
                _ => AllocationId::Input {
                    index,
                    origin: original.origin(),
                    may_alias: matches!(
                        original,
                        AllocationId::External(_)
                            | AllocationId::ExternalPath(_)
                            | AllocationId::ExternalBuffer(_)
                            | AllocationId::Input {
                                may_alias: true,
                                ..
                            }
                    ),
                },
            };
            objects.insert(original, mapped);
        }
        let state = project_state(caller, &map, &|id| objects.get(id).cloned());
        let args = args
            .iter()
            .enumerate()
            .map(|(index, arg)| {
                if !used(index) {
                    return Some(None);
                }
                let mut mapped = map.operand(arg)?;
                if let Operand::Constant(constant) = &mut mapped {
                    constant.span = rustc_span::DUMMY_SP;
                    constant.user_ty = None;
                }
                Some(Some(mapped))
            })
            .collect::<Option<_>>()?;
        let types = locals.iter().map(|local| decls[*local].ty).collect();
        Some(Self {
            input: InputContext { state, args, types },
            locals,
            objects: objects
                .into_iter()
                .map(|(original, mapped)| (mapped, original))
                .collect(),
            first_input,
        })
    }
}

struct ParameterUsage {
    args: HashSet<Local>,
    arg_count: usize,
    external: bool,
}

impl<'tcx> Visitor<'tcx> for ParameterUsage {
    fn visit_local(&mut self, local: Local, context: PlaceContext, _: Location) {
        if !matches!(context, PlaceContext::NonUse(_))
            && local.index() > 0
            && local.index() <= self.arg_count
        {
            self.args.insert(local);
        }
    }

    fn visit_terminator(&mut self, term: &rustc_middle::mir::Terminator<'tcx>, location: Location) {
        if matches!(
            term.kind,
            rustc_middle::mir::TerminatorKind::Call { .. }
                | rustc_middle::mir::TerminatorKind::Drop { .. }
                | rustc_middle::mir::TerminatorKind::InlineAsm { .. }
        ) {
            self.external = true;
        }
        self.super_terminator(term, location);
    }

    fn visit_const_operand(&mut self, constant: &ConstOperand<'tcx>, location: Location) {
        if matches!(
            constant.ty().kind(),
            ty::TyKind::RawPtr(..)
                | ty::TyKind::Ref(..)
                | ty::TyKind::Adt(..)
                | ty::TyKind::Array(..)
        ) {
            self.external = true;
        }
        self.super_const_operand(constant, location);
    }
}

struct Instantiate<'tcx> {
    tcx: TyCtxt<'tcx>,
    args: ty::GenericArgsRef<'tcx>,
}

impl<'tcx> MutVisitor<'tcx> for Instantiate<'tcx> {
    fn tcx(&self) -> TyCtxt<'tcx> {
        self.tcx
    }

    fn visit_ty(&mut self, ty: &mut Ty<'tcx>, _: TyContext) {
        *ty = ty::EarlyBinder::bind(*ty).instantiate(self.tcx, self.args);
        *ty = self
            .tcx
            .try_normalize_erasing_regions(TypingEnv::fully_monomorphized(), *ty)
            .unwrap_or(*ty);
    }

    fn visit_const_operand(&mut self, constant: &mut ConstOperand<'tcx>, _: Location) {
        constant.const_ = ty::EarlyBinder::bind(constant.const_).instantiate(self.tcx, self.args);
    }
}

pub(super) fn resolve_body<'tcx>(
    tcx: TyCtxt<'tcx>,
    func: &Operand<'tcx>,
    decls: &rustc_middle::mir::LocalDecls<'tcx>,
) -> Option<(Instance<'tcx>, Body<'tcx>)> {
    let ty::FnDef(def_id, args) = *func.ty(decls, tcx).kind() else {
        return None;
    };
    let instance =
        Instance::try_resolve(tcx, TypingEnv::fully_monomorphized(), def_id, args).ok()??;
    if !instance.def_id().is_local() || !tcx.is_mir_available(instance.def_id()) {
        return None;
    }
    let body = tcx.optimized_mir(instance.def_id());
    if body.basic_blocks.len() > 128 || body.local_decls.len() > 512 {
        return None;
    }
    let mut body = body.clone();
    Instantiate {
        tcx,
        args: instance.args,
    }
    .visit_body(&mut body);
    Some((instance, body))
}
