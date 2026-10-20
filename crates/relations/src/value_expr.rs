use rustc_middle::mir::{
    AggregateKind, BinOp, CastKind, LocalDecls, Operand, Place, ProjectionElem, Rvalue, UnOp,
};
use rustc_middle::ty::{Ty, TyCtxt, TyKind};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CheckedComponent {
    Value,
    Overflow,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ValueExpr<'tcx> {
    Use(Operand<'tcx>),
    Binary {
        op: BinOp,
        left: Operand<'tcx>,
        right: Operand<'tcx>,
    },
    CheckedBinary {
        op: BinOp,
        left: Operand<'tcx>,
        right: Operand<'tcx>,
        component: CheckedComponent,
    },
    Unary {
        op: UnOp,
        operand: Operand<'tcx>,
    },
    Cast {
        kind: CastKind,
        operand: Operand<'tcx>,
        target: Ty<'tcx>,
    },
}

impl Eq for ValueExpr<'_> {}

impl<'tcx> ValueExpr<'tcx> {
    pub fn map_operands(
        &self,
        map: &impl mirsa_framework::mapping::PathMapping<'tcx>,
    ) -> Option<Self> {
        Some(match self {
            Self::Use(operand) => Self::Use(map.operand(operand)?),
            Self::Binary { op, left, right } => Self::Binary {
                op: *op,
                left: map.operand(left)?,
                right: map.operand(right)?,
            },
            Self::CheckedBinary {
                op,
                left,
                right,
                component,
            } => Self::CheckedBinary {
                op: *op,
                left: map.operand(left)?,
                right: map.operand(right)?,
                component: *component,
            },
            Self::Unary { op, operand } => Self::Unary {
                op: *op,
                operand: map.operand(operand)?,
            },
            Self::Cast {
                kind,
                operand,
                target,
            } => Self::Cast {
                kind: *kind,
                operand: map.operand(operand)?,
                target: *target,
            },
        })
    }

    pub(crate) fn visit_operands(&self, mut visit: impl FnMut(&Operand<'tcx>)) {
        match self {
            Self::Binary { left, right, .. } | Self::CheckedBinary { left, right, .. } => {
                visit(left);
                visit(right);
            }
            Self::Use(operand) | Self::Unary { operand, .. } | Self::Cast { operand, .. } => {
                visit(operand)
            }
        }
    }
}

fn is_integer(ty: Ty<'_>) -> bool {
    matches!(ty.kind(), TyKind::Int(_) | TyKind::Uint(_))
}

fn is_scalar(ty: Ty<'_>) -> bool {
    is_integer(ty) || ty.is_bool() || ty.is_char() || ty.is_floating_point()
}

pub(crate) fn assignments<'tcx>(
    tcx: TyCtxt<'tcx>,
    local_decls: &LocalDecls<'tcx>,
    destination: Place<'tcx>,
    rvalue: &Rvalue<'tcx>,
) -> Vec<(Place<'tcx>, ValueExpr<'tcx>)> {
    if let Rvalue::Aggregate(kind, operands) = rvalue {
        let base = match kind.as_ref() {
            AggregateKind::Adt(def, variant, ..) if tcx.adt_def(*def).is_enum() => {
                destination.project_deeper(&[ProjectionElem::Downcast(None, *variant)], tcx)
            }
            _ => destination,
        };
        return operands
            .iter()
            .enumerate()
            .filter_map(|(index, operand)| {
                let projection = match kind.as_ref() {
                    AggregateKind::Tuple | AggregateKind::Closure(..) => {
                        ProjectionElem::Field(index.into(), operand.ty(local_decls, tcx))
                    }
                    AggregateKind::Adt(def, _, _, _, field) => {
                        if tcx.adt_def(*def).is_union() {
                            return None;
                        }
                        ProjectionElem::Field(
                            field.unwrap_or_else(|| index.into()),
                            operand.ty(local_decls, tcx),
                        )
                    }
                    AggregateKind::Array(_) => ProjectionElem::ConstantIndex {
                        offset: index as u64,
                        min_length: operands.len() as u64,
                        from_end: false,
                    },
                    _ => return None,
                };
                Some((
                    base.project_deeper(&[projection], tcx),
                    ValueExpr::Use(operand.clone()),
                ))
            })
            .collect();
    }
    let expr = match rvalue {
        Rvalue::BinaryOp(op, operands) => {
            let (left, right) = &**operands;
            if !is_scalar(left.ty(local_decls, tcx)) || !is_scalar(right.ty(local_decls, tcx)) {
                return Vec::new();
            }
            if matches!(
                op,
                BinOp::AddWithOverflow | BinOp::SubWithOverflow | BinOp::MulWithOverflow
            ) {
                return [CheckedComponent::Value, CheckedComponent::Overflow]
                    .into_iter()
                    .enumerate()
                    .map(|(index, component)| {
                        let ty = if component == CheckedComponent::Value {
                            left.ty(local_decls, tcx)
                        } else {
                            tcx.types.bool
                        };
                        (
                            destination
                                .project_deeper(&[ProjectionElem::Field(index.into(), ty)], tcx),
                            ValueExpr::CheckedBinary {
                                op: *op,
                                left: left.clone(),
                                right: right.clone(),
                                component,
                            },
                        )
                    })
                    .collect();
            }
            if !matches!(
                op,
                BinOp::Add
                    | BinOp::Sub
                    | BinOp::Mul
                    | BinOp::Div
                    | BinOp::Rem
                    | BinOp::BitXor
                    | BinOp::BitAnd
                    | BinOp::BitOr
                    | BinOp::Shl
                    | BinOp::Shr
            ) {
                return Vec::new();
            }
            ValueExpr::Binary {
                op: *op,
                left: left.clone(),
                right: right.clone(),
            }
        }
        Rvalue::UnaryOp(op @ (UnOp::Neg | UnOp::Not), operand)
            if is_scalar(operand.ty(local_decls, tcx)) =>
        {
            ValueExpr::Unary {
                op: *op,
                operand: operand.clone(),
            }
        }
        Rvalue::Cast(
            kind @ (CastKind::IntToInt
            | CastKind::IntToFloat
            | CastKind::FloatToInt
            | CastKind::FloatToFloat),
            operand,
            target,
        ) if is_scalar(operand.ty(local_decls, tcx)) && is_scalar(*target) => ValueExpr::Cast {
            kind: *kind,
            operand: operand.clone(),
            target: *target,
        },
        _ => return Vec::new(),
    };
    vec![(destination, expr)]
}
