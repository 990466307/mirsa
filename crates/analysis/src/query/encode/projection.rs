use super::super::value::Projection;
use super::*;

impl<'tcx> Encoder<'_, 'tcx> {
    fn select(
        &mut self,
        base: Place<'tcx>,
        index: Term,
        ty: ScalarType,
        depth: usize,
    ) -> Result<Term> {
        self.queue(base, depth);
        let path = self
            .canonical_path(base)
            .ok_or(UnknownReason::Unsupported)?;
        let key = (path, ty);
        let id = if let Some(id) = self.arrays.get(&key) {
            *id
        } else {
            let id = self.fresh()?;
            self.arrays.insert(key, id);
            id
        };
        let index_ty = abstract_query::scalar_type(self.tcx, self.tcx.types.usize).unwrap();
        Ok(Term::Select(
            id,
            Box::new(Term::Cast(Box::new(index), index_ty)),
            ty,
        ))
    }

    pub(super) fn projection(&mut self, expr: &Expr<'tcx>, depth: usize) -> Result<Term> {
        if let Some(place) = self.eval().place(expr) {
            return self.variable(place, false, depth);
        }
        if let Expr::TypedExpr(t) = expr {
            if let TypedKind::Project {
                base,
                projection: Projection::Index(index),
            } = &t.kind
            {
                let base = self.eval().place(base).ok_or(UnknownReason::Unsupported)?;
                let index_bounds = self.integer_bounds(index)?;
                let length = self.eval().eval(&Expr::Length(base));
                let (len, _) = length.integer().ok_or(UnknownReason::Unsupported)?;
                if index_bounds.0 < 0.into() || &index_bounds.1 >= len {
                    return Err(UnknownReason::Unsupported);
                }
                let index = Term::FromInteger(
                    Box::new(self.integer(index)?),
                    abstract_query::scalar_type(self.tcx, self.tcx.types.usize).unwrap(),
                );
                let ValueType::Rust(ty) = t.ty else {
                    return Err(UnknownReason::Unsupported);
                };
                let ty = self.storage_type(ty).ok_or(UnknownReason::Unsupported)?;
                return self.select(base, index, ty, depth);
            }
        }
        Err(UnknownReason::Unsupported)
    }

    pub(super) fn indexed_place(
        &mut self,
        place: Place<'tcx>,
        length: bool,
        depth: usize,
    ) -> Result<Term> {
        if length {
            return Err(UnknownReason::Unsupported);
        }
        let Some(ProjectionElem::Index(index)) = place.projection.last() else {
            return Err(UnknownReason::Unsupported);
        };
        let base = Place {
            local: place.local,
            projection: self
                .tcx
                .mk_place_elems(&place.projection[..place.projection.len() - 1]),
        };
        let ty = self
            .storage_type(place.ty(&self.body.local_decls, self.tcx).ty)
            .ok_or(UnknownReason::Unsupported)?;
        let index = self.variable(Place::from(*index), false, depth)?;
        self.select(base, index, ty, depth)
    }

    pub(super) fn element_fact(
        &mut self,
        place: Place<'tcx>,
        term: &Term,
        depth: usize,
    ) -> Result<()> {
        let Some(ProjectionElem::ConstantIndex {
            offset,
            from_end: false,
            ..
        }) = place.projection.last()
        else {
            return Ok(());
        };
        let base = Place {
            local: place.local,
            projection: self
                .tcx
                .mk_place_elems(&place.projection[..place.projection.len() - 1]),
        };
        let ty = term.scalar_type().ok_or(UnknownReason::Unsupported)?;
        let index = Term::Constant(
            *offset as u128,
            abstract_query::scalar_type(self.tcx, self.tcx.types.usize).unwrap(),
        );
        let selected = self.select(base, index, ty, depth)?;
        self.assumptions
            .push(Formula::Compare(Comparison::Eq, term.clone(), selected));
        Ok(())
    }
}
