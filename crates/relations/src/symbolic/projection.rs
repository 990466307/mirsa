use super::{SymbolicExpr, SymbolicFact, SymbolicState};
use mirsa_framework::mapping::PathMapping;
use rustc_middle::mir::Local;
use std::collections::HashSet;

impl<'tcx> SymbolicExpr<'tcx> {
    pub fn project(&self, map: &impl PathMapping<'tcx>) -> Option<Self> {
        Some(match self {
            Self::Pointer { base, offset } => Self::Pointer {
                base: map.operand(base)?,
                offset: match offset {
                    Some((kind, count)) => Some((*kind, map.operand(count)?)),
                    None => None,
                },
            },
            Self::PointerFromAddress { address } => Self::PointerFromAddress {
                address: map.operand(address)?,
            },
            Self::Value(value) => Self::Value(value.map_operands(map)?),
            Self::Cmp { op, left, right } => Self::Cmp {
                op: *op,
                left: map.operand(left)?,
                right: map.operand(right)?,
            },
            Self::Address { pointer } => Self::Address {
                pointer: map.operand(pointer)?,
            },
            Self::Length { place } => Self::Length {
                place: map.place(*place)?,
            },
            Self::Call { callee, args } => Self::Call {
                callee: *callee,
                args: args
                    .iter()
                    .map(|arg| map.operand(arg))
                    .collect::<Option<_>>()?,
            },
        })
    }
}

impl<'tcx> SymbolicState<'tcx> {
    pub fn project(&self, map: &impl PathMapping<'tcx>) -> Self {
        let mut out = Self::new_with_debug(self.debug);
        out.display_places = map.places(&self.display_places).into();
        let mut visited = HashSet::new();
        for path in self.eq.keys() {
            if visited.contains(&path) {
                continue;
            }
            let mut first: Option<mirsa_framework::access_path::AccessPath> = None;
            for member in self.eq.members(path) {
                visited.insert(member.clone());
                if let Some(mapped) = map.path(&member) {
                    if let Some(first) = &first {
                        out.eq.union(first.clone(), mapped);
                    } else {
                        first = Some(mapped);
                    }
                }
            }
        }
        let projected: Vec<_> = self
            .exprs
            .iter()
            .filter_map(|(path, expr)| Some((path, self.project_expression(expr, map)?)))
            .collect();
        for (path, expr) in &projected {
            if let Some(path) = map.path(path) {
                out.exprs.insert(path, expr.clone());
            }
        }
        for (path, expr) in &projected {
            for equivalent in self
                .eq
                .members((*path).clone())
                .take(mirsa_framework::reduction::MAX_RELATED)
            {
                if let Some(mapped) = map.path(&equivalent) {
                    out.exprs.entry(mapped).or_insert_with(|| expr.clone());
                }
            }
        }
        let expressions = std::mem::take(&mut out.exprs);
        for (path, expr) in &expressions {
            out.index_expr(path, expr);
        }
        out.exprs = expressions;
        for fact in &self.facts {
            let (expr, value) = match fact {
                SymbolicFact::EqConst { expr, value } | SymbolicFact::NeConst { expr, value } => {
                    (expr, *value)
                }
            };
            if let Some(expr) = map.operand(expr) {
                out.push_fact(match fact {
                    SymbolicFact::EqConst { .. } => SymbolicFact::EqConst { expr, value },
                    SymbolicFact::NeConst { .. } => SymbolicFact::NeConst { expr, value },
                });
            }
        }
        for fact in &self.predicates {
            if let Some(fact) = fact.project(map) {
                out.add_predicate(fact);
            }
        }
        out.points_to = self
            .points_to
            .iter()
            .filter_map(|(pointer, pointee)| Some((map.path(pointer)?, map.path(pointee)?)))
            .collect();
        out.address_taken = map.paths(&self.address_taken).into();
        out.escaped = map.paths(&self.escaped).into();
        out
    }

    fn project_expression(
        &self,
        expr: &SymbolicExpr<'tcx>,
        map: &impl PathMapping<'tcx>,
    ) -> Option<SymbolicExpr<'tcx>> {
        if let Some(expr) = expr.project(map) {
            return Some(expr);
        }
        let SymbolicExpr::Pointer { base, offset } = expr else {
            return None;
        };
        let mut base = base.clone();
        let mut offset = offset.clone();
        for _ in 0..mirsa_framework::reduction::MAX_DEPTH {
            let SymbolicExpr::Pointer {
                base: source,
                offset: source_offset,
            } = self.expr_for_place(base.place()?)?
            else {
                return None;
            };
            if offset.is_some() && source_offset.is_some() {
                return None;
            }
            offset = offset.or_else(|| source_offset.clone());
            base = source.clone();
            if let Some(expr) = (SymbolicExpr::Pointer {
                base: base.clone(),
                offset: offset.clone(),
            })
            .project(map)
            {
                return Some(expr);
            }
        }
        None
    }

    pub fn extend(&mut self, other: Self) {
        for path in other.eq.keys() {
            if let Some(first) = other.eq.members(path.clone()).next() {
                self.eq.union(path, first);
            }
        }
        self.display_places.extend(other.display_places);
        for (path, expr) in other.exprs {
            self.remove_expr(&path);
            self.index_expr(&path, &expr);
            self.exprs.insert(path, expr);
        }
        for fact in other.facts {
            self.push_fact(fact);
        }
        for fact in other.predicates {
            self.add_predicate(fact);
        }
        self.points_to.extend(other.points_to);
        self.address_taken.extend(other.address_taken);
        self.escaped.extend(other.escaped);
    }

    pub fn related_locals(&self, roots: &HashSet<Local>, include_escaped: bool) -> HashSet<Local> {
        let mut out = roots.clone();
        if include_escaped {
            out.extend(self.escaped.iter().map(|path| path.root));
        }
        for (path, expr) in &self.exprs {
            let mut operands = Vec::new();
            expr.visit_places(|place| operands.push(place.local));
            if roots.contains(&path.root) || operands.iter().any(|local| roots.contains(local)) {
                out.insert(path.root);
                out.extend(operands);
            }
        }
        for fact in &self.predicates {
            let locals: Vec<_> = fact
                .values
                .values()
                .filter_map(|v| v.place().map(|p| p.local))
                .collect();
            if locals.iter().any(|local| roots.contains(local)) {
                out.extend(locals);
            }
        }
        for (pointer, pointee) in &self.points_to {
            if roots.contains(&pointer.root) {
                out.insert(pointee.root);
            }
        }
        for path in self.eq.keys().filter(|path| roots.contains(&path.root)) {
            out.extend(self.eq.members(path).map(|path| path.root));
        }
        out
    }
}
