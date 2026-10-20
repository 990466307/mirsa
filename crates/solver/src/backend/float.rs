use super::*;
use crate::FloatTerm;
use z3::{
    Sort,
    ast::{Float, Real},
};
use z3_sys::*;

impl<'ctx> Encoder<'ctx> {
    fn float_sort(&self, bits: u32) -> Option<Sort<'ctx>> {
        Some(match bits {
            32 => Sort::float32(self.ctx),
            64 => Sort::double(self.ctx),
            _ => return None,
        })
    }

    fn nearest(&self) -> Float<'ctx> {
        unsafe {
            Float::wrap(
                self.ctx,
                Z3_mk_fpa_round_nearest_ties_to_even(self.ctx.get_z3_context()),
            )
        }
    }

    fn float_real(&self, value: &Float<'ctx>) -> Real<'ctx> {
        unsafe {
            Real::wrap(
                self.ctx,
                Z3_mk_fpa_to_real(self.ctx.get_z3_context(), value.get_z3_ast()),
            )
        }
    }

    fn real_float(&self, value: &Real<'ctx>, bits: u32) -> Option<Float<'ctx>> {
        let sort = self.float_sort(bits)?;
        Some(unsafe {
            Float::wrap(
                self.ctx,
                Z3_mk_fpa_to_fp_real(
                    self.ctx.get_z3_context(),
                    self.nearest().get_z3_ast(),
                    value.get_z3_ast(),
                    sort.get_z3_sort(),
                ),
            )
        })
    }

    fn truncate(&self, value: &Real<'ctx>) -> Int<'ctx> {
        let zero = Real::from_real(self.ctx, 0, 1);
        value
            .ge(&zero)
            .ite(&value.to_int(), &value.unary_minus().to_int().unary_minus())
    }

    pub(super) fn float(&mut self, term: &FloatTerm) -> Option<Float<'ctx>> {
        let bits = term.bits()?;
        let sort = self.float_sort(bits)?;
        let ctx = self.ctx.get_z3_context();
        Some(match term {
            FloatTerm::FromMath(value, _) => {
                let value = self.integer(value)?.to_real();
                self.real_float(&value, bits)?
            }
            FloatTerm::FromBits(value) => {
                let value = self.term(value)?;
                unsafe {
                    Float::wrap(
                        self.ctx,
                        Z3_mk_fpa_to_fp_bv(ctx, value.get_z3_ast(), sort.get_z3_sort()),
                    )
                }
            }
            FloatTerm::FromInteger(value, _) => {
                let signed = value.scalar_type()?.signed;
                let value = self.term(value)?;
                let rm = self.nearest();
                unsafe {
                    Float::wrap(
                        self.ctx,
                        if signed {
                            Z3_mk_fpa_to_fp_signed(
                                ctx,
                                rm.get_z3_ast(),
                                value.get_z3_ast(),
                                sort.get_z3_sort(),
                            )
                        } else {
                            Z3_mk_fpa_to_fp_unsigned(
                                ctx,
                                rm.get_z3_ast(),
                                value.get_z3_ast(),
                                sort.get_z3_sort(),
                            )
                        },
                    )
                }
            }
            FloatTerm::Cast(value, _) => {
                let value = self.float(value)?;
                unsafe {
                    Float::wrap(
                        self.ctx,
                        Z3_mk_fpa_to_fp_float(
                            ctx,
                            self.nearest().get_z3_ast(),
                            value.get_z3_ast(),
                            sort.get_z3_sort(),
                        ),
                    )
                }
            }
            FloatTerm::Neg(value) => self.float(value)?.unary_neg(),
            FloatTerm::Binary(op, a, b) => {
                let a = self.float(a)?;
                let b = self.float(b)?;
                let rm = self.nearest();
                match op {
                    BinaryOp::Add => rm.add(&a, &b),
                    BinaryOp::Sub => rm.sub(&a, &b),
                    BinaryOp::Mul => rm.mul(&a, &b),
                    BinaryOp::Div => rm.div(&a, &b),
                    BinaryOp::Rem => {
                        let x = self.float_real(&a);
                        let y = self.float_real(&b);
                        let q = self.truncate(&x.div(&y)).to_real();
                        let r = Real::sub(self.ctx, &[&x, &Real::mul(self.ctx, &[&y, &q])]);
                        let result = self.real_float(&r, bits)?;
                        let positive_zero = if bits == 32 {
                            Float::from_f32(self.ctx, 0.0)
                        } else {
                            Float::from_f64(self.ctx, 0.0)
                        };
                        let nan = if bits == 32 {
                            Float::from_f32(self.ctx, f32::NAN)
                        } else {
                            Float::from_f64(self.ctx, f64::NAN)
                        };
                        let sign = unsafe {
                            Bool::wrap(self.ctx, Z3_mk_fpa_is_negative(ctx, a.get_z3_ast()))
                        };
                        let zero = sign.ite(&positive_zero.unary_neg(), &positive_zero);
                        let result = result.is_zero().ite(&zero, &result);
                        let invalid = Bool::or(
                            self.ctx,
                            &[&a.is_nan(), &b.is_nan(), &a.is_infinite(), &b.is_zero()],
                        );
                        invalid.ite(&nan, &b.is_infinite().ite(&a, &result))
                    }
                    _ => return None,
                }
            }
        })
    }

    pub(super) fn float_compare(
        &mut self,
        op: Comparison,
        a: &FloatTerm,
        b: &FloatTerm,
    ) -> Option<Bool<'ctx>> {
        if a.bits()? != b.bits()? {
            return None;
        }
        let a = self.float(a)?;
        let b = self.float(b)?;
        Some(match op {
            Comparison::Eq | Comparison::Ne => {
                let eq = unsafe {
                    Bool::wrap(
                        self.ctx,
                        Z3_mk_fpa_eq(self.ctx.get_z3_context(), a.get_z3_ast(), b.get_z3_ast()),
                    )
                };
                if op == Comparison::Eq { eq } else { eq.not() }
            }
            Comparison::Lt => a.lt(&b),
            Comparison::Le => a.le(&b),
            Comparison::Gt => a.gt(&b),
            Comparison::Ge => a.ge(&b),
        })
    }

    pub(super) fn float_to_integer(
        &mut self,
        value: &FloatTerm,
        ty: ScalarType,
    ) -> Option<BV<'ctx>> {
        let value = self.float(value)?;
        let min = self.constant(if ty.signed { 1u128 << (ty.bits - 1) } else { 0 }, ty);
        let max = self.constant(
            if ty.signed {
                (1u128 << (ty.bits - 1)) - 1
            } else {
                u128::MAX
            },
            ty,
        );
        let n = self.truncate(&self.float_real(&value));
        let low = Int::from_bv(&min, ty.signed);
        let high = Int::from_bv(&max, ty.signed);
        let finite = n
            .lt(&low)
            .ite(&min, &n.gt(&high).ite(&max, &BV::from_int(&n, ty.bits)));
        let sign = unsafe {
            Bool::wrap(
                self.ctx,
                Z3_mk_fpa_is_negative(self.ctx.get_z3_context(), value.get_z3_ast()),
            )
        };
        Some(value.is_nan().ite(
            &self.constant(0, ty),
            &value.is_infinite().ite(&sign.ite(&min, &max), &finite),
        ))
    }
}
