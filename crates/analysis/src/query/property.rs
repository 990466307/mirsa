pub use mirsa_solver::{BinaryOp, Comparison, UnaryOp};
use rustc_middle::mir::{Operand, Place};
use rustc_middle::ty::Ty;

#[derive(Clone, Debug)]
pub(super) enum Expr<'tcx> {
    Value(Operand<'tcx>),
    Constant { bits: u128, ty: Ty<'tcx> },
    Length(Place<'tcx>),
    TypedExpr(Box<super::value::TypedExpr<'tcx>>),
    Binary(BinaryOp, Box<Self>, Box<Self>),
    Unary(UnaryOp, Box<Self>),
    Cast(Box<Self>, Ty<'tcx>),
}

impl<'tcx> Expr<'tcx> {
    pub(super) fn visit_exprs(&self, visit: &mut impl FnMut(&Expr<'tcx>)) {
        match self {
            Self::Binary(_, a, b) => {
                a.visit_exprs(visit);
                b.visit_exprs(visit);
            }
            Self::Unary(_, a) | Self::Cast(a, _) => a.visit_exprs(visit),
            Self::TypedExpr(v) => {
                for child in v.children() {
                    child.visit_exprs(visit);
                }
                if let super::value::TypedKind::Boolean(p) = &v.kind {
                    p.visit_exprs(visit);
                }
            }
            _ => {}
        }
        visit(self);
    }

    pub(super) fn visit_places(&self, visit: &mut impl FnMut(Place<'tcx>)) {
        match self {
            Self::Value(value) => {
                if let Some(place) = value.place() {
                    visit(place);
                }
            }
            Self::Length(place) => visit(*place),
            Self::Binary(_, a, b) => {
                a.visit_places(visit);
                b.visit_places(visit);
            }
            Self::Unary(_, value) | Self::Cast(value, _) => value.visit_places(visit),
            Self::TypedExpr(value) => value.visit_places(visit),
            Self::Constant { .. } => {}
        }
    }
}

#[derive(Clone, Debug)]
pub(super) enum Predicate<'tcx> {
    Value(Expr<'tcx>),
    Compare(Comparison, Expr<'tcx>, Expr<'tcx>),
    Not(Box<Self>),
    All(Vec<Self>),
    Any(Vec<Self>),
}

impl<'tcx> Predicate<'tcx> {
    pub(super) fn visit_exprs(&self, visit: &mut impl FnMut(&Expr<'tcx>)) {
        match self {
            Self::Value(e) => e.visit_exprs(visit),
            Self::Compare(_, a, b) => {
                a.visit_exprs(visit);
                b.visit_exprs(visit);
            }
            Self::Not(p) => p.visit_exprs(visit),
            Self::All(values) | Self::Any(values) => {
                for p in values {
                    p.visit_exprs(visit);
                }
            }
        }
    }

    pub(super) fn within_budget(&self, max_nodes: usize) -> bool {
        enum Node<'a, 'tcx> {
            Property(&'a Predicate<'tcx>),
            Expr(&'a Expr<'tcx>),
        }
        let mut pending = vec![(Node::Property(self), 0)];
        let mut count = 0;
        while let Some((node, depth)) = pending.pop() {
            count += 1;
            if count > max_nodes || depth > 64 {
                return false;
            }
            match node {
                Node::Property(property) => match property {
                    Self::Compare(_, a, b) => {
                        pending.push((Node::Expr(a), depth + 1));
                        pending.push((Node::Expr(b), depth + 1));
                    }
                    Self::Value(value) => pending.push((Node::Expr(value), depth + 1)),
                    Self::Not(value) => pending.push((Node::Property(value), depth + 1)),
                    Self::All(values) | Self::Any(values) => {
                        if values.len() > max_nodes {
                            return false;
                        }
                        pending.extend(
                            values
                                .iter()
                                .map(|value| (Node::Property(value), depth + 1)),
                        );
                    }
                },
                Node::Expr(expr) => match expr {
                    Expr::Binary(_, a, b) => {
                        pending.push((Node::Expr(a), depth + 1));
                        pending.push((Node::Expr(b), depth + 1));
                    }
                    Expr::Unary(_, value) | Expr::Cast(value, _) => {
                        pending.push((Node::Expr(value), depth + 1))
                    }
                    Expr::TypedExpr(value) => {
                        for child in value.children() {
                            pending.push((Node::Expr(child), depth + 1));
                        }
                        if let super::value::TypedKind::Boolean(p) = &value.kind {
                            pending.push((Node::Property(p), depth + 1));
                        }
                    }
                    _ => {}
                },
            }
        }
        true
    }

    pub(super) fn visit_places(&self, visit: &mut impl FnMut(Place<'tcx>)) {
        match self {
            Self::Value(value) => value.visit_places(visit),
            Self::Compare(_, a, b) => {
                a.visit_places(visit);
                b.visit_places(visit);
            }
            Self::Not(value) => value.visit_places(visit),
            Self::All(values) | Self::Any(values) => {
                for value in values {
                    value.visit_places(visit);
                }
            }
        }
    }
}
