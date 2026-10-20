//! Shared query roots and attribute requirements, independent of solver encoding.
use super::{
    Expr, Predicate,
    value::{Projection, TypedKind},
};
use mirsa_dsl::Builtin;
use rustc_middle::mir::{Local, ProjectionElem};
use std::collections::HashSet;

#[derive(Default)]
pub(crate) struct Demand {
    pub locals: HashSet<Local>,
    pub memory: bool,
}
impl Demand {
    pub(super) fn expression(&mut self, expression: &Expr<'_>) {
        expression.visit_places(&mut |p| {
            self.locals.insert(p.local);
            for projection in p.projection {
                match projection {
                    ProjectionElem::Index(local) => {
                        self.locals.insert(local);
                        self.memory = true;
                    }
                    ProjectionElem::Deref => self.memory = true,
                    _ => {}
                }
            }
        });
        expression.visit_exprs(&mut |e| {
            if let Expr::TypedExpr(t) = e {
                match &t.kind {
                    TypedKind::Builtin(
                        Builtin::Base
                        | Builtin::Offset
                        | Builtin::Extent
                        | Builtin::IsLive
                        | Builtin::IsFreed
                        | Builtin::SameAllocation,
                        _,
                    ) => self.memory = true,
                    TypedKind::Project {
                        projection: Projection::Deref | Projection::Index(_),
                        ..
                    } => self.memory = true,
                    _ => {}
                }
            }
        });
    }
    pub(super) fn predicate(property: &Predicate<'_>) -> Self {
        let mut demand = Self::default();
        property.visit_exprs(&mut |e| demand.expression(e));
        demand
    }
}
