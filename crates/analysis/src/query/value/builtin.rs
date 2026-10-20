use super::*;

pub(crate) fn builtin<'tcx>(
    tcx: TyCtxt<'tcx>,
    body: &Body<'tcx>,
    op: Builtin,
    args: Vec<Expr<'tcx>>,
) -> Result<Expr<'tcx>, &'static str> {
    use Builtin::*;
    if args.len() != op.arity() {
        return Err("incorrect builtin argument count");
    }
    let input = ty(tcx, body, &args[0]);
    let pointer = |t| matches!(t, ValueType::Rust(t) if is_allocation_pointer(tcx, t));
    let boolean = ValueType::Rust(tcx.types.bool);
    let output = match op {
        Math if input == ValueType::Math
            || matches!(input, ValueType::Rust(t) if t.is_integral()) =>
        {
            ValueType::Math
        }
        Math => return Err("math requires an integer"),
        Len => {
            let sequence = |t: Ty<'tcx>| {
                matches!(
                    t.kind(),
                    TyKind::Array(..) | TyKind::Slice(..) | TyKind::Str
                )
            };
            let valid = matches!(input, ValueType::Rust(t) if sequence(t)
                || matches!(t.kind(), TyKind::Ref(_, inner, _) if sequence(*inner)));
            if !valid {
                return Err("len requires an array, slice or str");
            }
            if let Expr::Value(v) = &args[0] {
                if let Some(p) = v.place() {
                    return Ok(Expr::Length(p));
                }
            }
            ValueType::Rust(tcx.types.usize)
        }
        IsNan | IsInfinite => {
            if !matches!(input, ValueType::Rust(t) if t.is_floating_point()) {
                return Err("float builtin requires f32 or f64");
            }
            boolean
        }
        IsNull | IsLive | IsFreed | SameAllocation => {
            if !pointer(input) {
                return Err("memory builtin requires a pointer");
            }
            if op == SameAllocation && !pointer(ty(tcx, body, &args[1])) {
                return Err("same_allocation requires two pointers");
            }
            boolean
        }
        Offset | Extent | Address => {
            if !pointer(input) {
                return Err("pointer builtin requires a pointer");
            }
            ValueType::Math
        }
        Base => {
            if !pointer(input)
                && !matches!(args[0], Expr::Value(Operand::Copy(_) | Operand::Move(_)))
                && !matches!(&args[0], Expr::TypedExpr(t) if matches!(t.kind, TypedKind::Project { .. }))
            {
                return Err("base requires an object place or pointer");
            }
            ValueType::Rust(Ty::new_ptr(tcx, tcx.types.u8, rustc_hir::Mutability::Not))
        }
        LayoutSize | LayoutAlign => {
            if !matches!(input, ValueType::Rust(t) if mirsa_domains::allocation::state::is_layout_ty(tcx, t))
            {
                return Err("layout builtin requires Layout");
            }
            ValueType::Math
        }
        Discriminant => {
            let ValueType::Rust(t) = input else {
                return Err("discriminant requires an enum");
            };
            if !matches!(t.kind(), TyKind::Adt(adt, _) if adt.is_enum()) {
                return Err("discriminant requires an enum");
            }
            ValueType::Rust(t.discriminant_ty(tcx))
        }
    };
    Ok(typed(output, TypedKind::Builtin(op, args)))
}

impl<'tcx> Eval<'_, 'tcx> {
    pub fn address_place(&self, pointer: &Expr<'tcx>) -> Option<Place<'tcx>> {
        let p = self.place(pointer)?;
        let mut pending = vec![p];
        let mut index = 0;
        while index < pending.len() && index < 32 {
            let candidate = pending[index];
            index += 1;
            if let Some(mirsa_relations::symbolic::SymbolicExpr::Address { pointer }) =
                self.state.symbolic.expr_for_place(candidate)
            {
                if let Some(other) = pointer.place() {
                    if self.state.symbolic.equiv_places_readonly(p, other) {
                        return Some(candidate);
                    }
                }
            }
            for related in self.state.symbolic.related_places(candidate, 32) {
                if pending.len() < 32 && !pending.contains(&related) {
                    pending.push(related);
                }
            }
        }
        None
    }

    pub fn scalar_place(&self, expr: &Expr<'tcx>) -> Option<Place<'tcx>> {
        if let Expr::TypedExpr(e) = expr {
            if let TypedKind::Builtin(op, args) = &e.kind {
                if *op == Builtin::Math {
                    return self.scalar_place(&args[0]);
                }
                if *op == Builtin::Address {
                    return self.address_place(&args[0]);
                }
            }
        }
        let p = self.place(expr)?;
        abstract_query::scalar_type(self.tcx, p.ty(&self.body.local_decls, self.tcx).ty)?;
        Some(p)
    }

    pub(super) fn eval_builtin(&self, op: Builtin, args: &[Expr<'tcx>]) -> Value {
        use Builtin::*;
        if op == Len {
            return self
                .place(&args[0])
                .map(|p| self.length(p))
                .unwrap_or(Value::Unknown);
        }
        if matches!(op, LayoutSize | LayoutAlign | Discriminant) {
            let Some(p) = self.place(&args[0]) else {
                return Value::Unknown;
            };
            if op == Discriminant {
                let Some(index) = self
                    .state
                    .interval
                    .enum_variant_resolved(&self.state.symbolic, p)
                else {
                    return Value::Unknown;
                };
                let TyKind::Adt(adt, _) = p.ty(&self.body.local_decls, self.tcx).ty.kind() else {
                    return Value::Unknown;
                };
                let d = adt.discriminant_for_variant(self.tcx, index.into());
                return raw_integer(self.tcx, d.ty, d.val);
            }
            let l = self
                .state
                .allocation
                .layout_value_resolved(&self.state.symbolic, p);
            return Value::range(if op == LayoutSize { l.size } else { l.align });
        }
        if op == Base {
            let pointer = match self.eval(&args[0]) {
                Value::Pointer(p) => p,
                _ => {
                    let Some(p) = self.place(&args[0]) else {
                        return Value::Unknown;
                    };
                    if let Some(index) = p
                        .projection
                        .iter()
                        .rposition(|p| matches!(p, ProjectionElem::Deref))
                    {
                        let pointer = Place {
                            local: p.local,
                            projection: self.tcx.mk_place_elems(&p.projection[..index]),
                        };
                        self.state
                            .allocation
                            .pointer_value_resolved(&self.state.symbolic, pointer)
                    } else {
                        let id = AllocationId::Stack(p.local);
                        if self.state.allocation.object(&id).is_none() {
                            return Value::Unknown;
                        }
                        PointerValue::target(id, Interval::new(0, 0))
                    }
                }
            };
            if pointer.has_unknown_target()
                || pointer.may_be_null()
                || pointer.may_be_invalid_non_null()
            {
                return Value::Unknown;
            }
            let mut out = PointerValue::bottom();
            for (id, _) in pointer.targets() {
                out = PointerValue::join(
                    &out,
                    &PointerValue::target(id.clone(), Interval::new(0, 0)),
                );
            }
            return Value::Pointer(out);
        }
        if op == Address {
            if let Some(place) = self.address_place(&args[0]) {
                return self.read(place);
            }
        }
        let value = self.eval(&args[0]);
        match (op, value) {
            (Math, v @ Value::Integer(..)) => v,
            (IsNan, Value::Float(f)) => Value::Bool(if !f.may_nan {
                Some(false)
            } else if !f.has_numeric_values() {
                Some(true)
            } else {
                None
            }),
            (IsInfinite, Value::Float(f)) => Value::Bool(
                if !f.contains_pos_infinity() && !f.contains_neg_infinity() {
                    Some(false)
                } else if !f.may_nan && !f.has_finite_values() {
                    Some(true)
                } else {
                    None
                },
            ),
            (IsNull, Value::Pointer(p)) => Value::Bool(if p.is_null_only() {
                Some(true)
            } else if p.is_definitely_non_null() {
                Some(false)
            } else {
                None
            }),
            (IsLive, Value::Pointer(p)) => {
                Value::Bool(abstract_bool(self.state.allocation.is_live(&p)))
            }
            (IsFreed, Value::Pointer(p)) => {
                if p.has_unknown_target() {
                    return Value::Bool(None);
                }
                let mut all_true =
                    !p.may_be_null() && !p.may_be_invalid_non_null() && p.target_count() > 0;
                let mut all_false = true;
                for (id, _) in p.targets() {
                    let Some(f) = self.state.allocation.object(id) else {
                        return Value::Bool(None);
                    };
                    all_true &= f.status.is_definitely_dead();
                    all_false &= !f.status.may_be_dead();
                }
                Value::Bool(if all_true {
                    Some(true)
                } else if all_false {
                    Some(false)
                } else {
                    None
                })
            }
            (SameAllocation, Value::Pointer(a)) => match self.eval(&args[1]) {
                Value::Pointer(b) => {
                    Value::Bool(abstract_bool(self.state.allocation.same_allocation(&a, &b)))
                }
                _ => Value::Bool(None),
            },
            (Offset | Extent, Value::Pointer(p)) => {
                if p.has_unknown_target() || p.may_be_null() || p.may_be_invalid_non_null() {
                    return Value::Unknown;
                }
                let mut range = None;
                for (id, off) in p.targets() {
                    let r = if op == Offset {
                        *off
                    } else {
                        let Some(f) = self.state.allocation.object(id) else {
                            return Value::Unknown;
                        };
                        f.extent
                    };
                    range = Some(match range {
                        None => r,
                        Some(prev) => mirsa_domains::interval::abstract_value::join(&prev, &r),
                    });
                }
                range.map(Value::range).unwrap_or(Value::Unknown)
            }
            (Address, Value::Pointer(p)) => {
                if p.is_null_only() {
                    Value::constant(0)
                } else {
                    let (_, max) = bounds(self.tcx, self.tcx.types.usize).unwrap();
                    Value::Integer(
                        if p.is_definitely_non_null() {
                            1.into()
                        } else {
                            0.into()
                        },
                        max,
                    )
                }
            }
            _ => Value::Unknown,
        }
    }
}
