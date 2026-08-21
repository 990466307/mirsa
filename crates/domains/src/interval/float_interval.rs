use super::abstract_value::{Interval, join as join_integer};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FloatKind {
    F32,
    F64,
}

impl FloatKind {
    pub fn round(self, value: f64) -> f64 {
        match self {
            Self::F32 => f64::from(value as f32),
            Self::F64 => value,
        }
    }
}

/// An IEEE-754-aware interval.
///
/// `low..=high` describes all non-NaN values and `may_nan` tracks NaN
/// separately because NaN is unordered and cannot be represented by numeric
/// endpoints.  Empty numeric bounds together with `may_nan == false` are
/// bottom; empty numeric bounds together with `may_nan == true` mean NaN only.
#[derive(Clone, Copy, Debug)]
pub struct FloatInterval {
    pub low: f64,
    pub high: f64,
    pub may_nan: bool,
}

impl PartialEq for FloatInterval {
    fn eq(&self, other: &Self) -> bool {
        self.low == other.low && self.high == other.high && self.may_nan == other.may_nan
    }
}

impl Eq for FloatInterval {}

impl std::fmt::Display for FloatInterval {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.is_bottom() {
            return write!(f, "∅");
        }
        if !self.has_numeric_values() {
            return write!(f, "{{NaN}}");
        }

        let low = format_bound(self.low);
        let high = format_bound(self.high);
        if self.may_nan {
            write!(f, "[{low}, {high}] ∪ {{NaN}}")
        } else {
            write!(f, "[{low}, {high}]")
        }
    }
}

impl FloatInterval {
    pub fn new(low: f64, high: f64, may_nan: bool) -> Self {
        if low.is_nan() || high.is_nan() {
            return Self::top();
        }
        if low > high {
            return if may_nan {
                Self::nan_only()
            } else {
                Self::bottom()
            };
        }
        Self {
            low: canonicalize_zero(low),
            high: canonicalize_zero(high),
            may_nan,
        }
    }

    pub fn numeric(low: f64, high: f64) -> Self {
        Self::new(low, high, false)
    }

    pub fn singleton(value: f64) -> Self {
        if value.is_nan() {
            Self::nan_only()
        } else {
            Self::numeric(value, value)
        }
    }

    pub fn bottom() -> Self {
        Self {
            low: f64::INFINITY,
            high: f64::NEG_INFINITY,
            may_nan: false,
        }
    }

    pub fn nan_only() -> Self {
        Self {
            low: f64::INFINITY,
            high: f64::NEG_INFINITY,
            may_nan: true,
        }
    }

    pub fn top() -> Self {
        Self {
            low: f64::NEG_INFINITY,
            high: f64::INFINITY,
            may_nan: true,
        }
    }

    pub fn has_numeric_values(self) -> bool {
        self.low <= self.high
    }

    pub fn is_bottom(self) -> bool {
        !self.has_numeric_values() && !self.may_nan
    }

    pub fn is_numeric_singleton(self) -> bool {
        self.has_numeric_values() && !self.may_nan && self.low == self.high
    }

    pub fn contains_zero(self) -> bool {
        self.has_numeric_values() && self.low <= 0.0 && self.high >= 0.0
    }

    pub fn contains_pos_infinity(self) -> bool {
        self.has_numeric_values() && self.high == f64::INFINITY
    }

    pub fn contains_neg_infinity(self) -> bool {
        self.has_numeric_values() && self.low == f64::NEG_INFINITY
    }

    pub fn has_finite_values(self) -> bool {
        self.has_numeric_values() && self.low != f64::INFINITY && self.high != f64::NEG_INFINITY
    }

    pub fn without_nan(self) -> Self {
        Self {
            may_nan: false,
            ..self
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FloatCmpOp {
    Lt,
    Le,
    Gt,
    Ge,
    Eq,
    Ne,
}

pub fn intersect(a: &FloatInterval, b: &FloatInterval) -> FloatInterval {
    let may_nan = a.may_nan && b.may_nan;
    if !a.has_numeric_values() || !b.has_numeric_values() {
        return FloatInterval::new(f64::INFINITY, f64::NEG_INFINITY, may_nan);
    }
    FloatInterval::new(a.low.max(b.low), a.high.min(b.high), may_nan)
}

pub fn join(a: &FloatInterval, b: &FloatInterval) -> FloatInterval {
    if a.is_bottom() {
        return *b;
    }
    if b.is_bottom() {
        return *a;
    }

    let may_nan = a.may_nan || b.may_nan;
    match (a.has_numeric_values(), b.has_numeric_values()) {
        (true, true) => FloatInterval::new(a.low.min(b.low), a.high.max(b.high), may_nan),
        (true, false) => FloatInterval::new(a.low, a.high, may_nan),
        (false, true) => FloatInterval::new(b.low, b.high, may_nan),
        (false, false) => FloatInterval::new(f64::INFINITY, f64::NEG_INFINITY, may_nan),
    }
}

pub fn widen(previous: &FloatInterval, next: &FloatInterval) -> FloatInterval {
    if previous.is_bottom() {
        return *next;
    }
    if next.is_bottom() {
        return *previous;
    }

    let may_nan = previous.may_nan || next.may_nan;
    match (previous.has_numeric_values(), next.has_numeric_values()) {
        (true, true) => {
            let low = if next.low < previous.low {
                f64::NEG_INFINITY
            } else {
                previous.low
            };
            let high = if next.high > previous.high {
                f64::INFINITY
            } else {
                previous.high
            };
            FloatInterval::new(low, high, may_nan)
        }
        (true, false) => FloatInterval::new(previous.low, previous.high, may_nan),
        (false, true) => FloatInterval::new(next.low, next.high, may_nan),
        (false, false) => FloatInterval::new(f64::INFINITY, f64::NEG_INFINITY, may_nan),
    }
}

pub fn add(kind: FloatKind, a: &FloatInterval, b: &FloatInterval) -> FloatInterval {
    let invalid = (a.contains_pos_infinity() && b.contains_neg_infinity())
        || (a.contains_neg_infinity() && b.contains_pos_infinity());
    binary_hull(kind, a, b, eval_add, invalid)
}

pub fn sub(kind: FloatKind, a: &FloatInterval, b: &FloatInterval) -> FloatInterval {
    let invalid = (a.contains_pos_infinity() && b.contains_pos_infinity())
        || (a.contains_neg_infinity() && b.contains_neg_infinity());
    binary_hull(kind, a, b, eval_sub, invalid)
}

pub fn neg(a: &FloatInterval) -> FloatInterval {
    if !a.has_numeric_values() {
        return FloatInterval::new(f64::INFINITY, f64::NEG_INFINITY, a.may_nan);
    }
    FloatInterval::new(-a.high, -a.low, a.may_nan)
}

pub fn mul(kind: FloatKind, a: &FloatInterval, b: &FloatInterval) -> FloatInterval {
    let invalid = (a.contains_zero() && (b.contains_pos_infinity() || b.contains_neg_infinity()))
        || (b.contains_zero() && (a.contains_pos_infinity() || a.contains_neg_infinity()));
    let mut result = binary_hull(kind, a, b, eval_mul, invalid);
    if (a.contains_zero() && b.has_finite_values()) || (b.contains_zero() && a.has_finite_values())
    {
        result = join(&result, &FloatInterval::singleton(0.0));
    }
    result
}

pub fn div(kind: FloatKind, a: &FloatInterval, b: &FloatInterval) -> FloatInterval {
    if a.is_bottom() || b.is_bottom() {
        return FloatInterval::bottom();
    }
    if !a.has_numeric_values() || !b.has_numeric_values() {
        return FloatInterval::nan_only();
    }
    if b.contains_zero() {
        // The sign of zero and values arbitrarily close to zero are not
        // represented.  Top is the sound interval result here.
        return FloatInterval::top();
    }
    let invalid = (a.contains_pos_infinity() || a.contains_neg_infinity())
        && (b.contains_pos_infinity() || b.contains_neg_infinity());
    let mut result = binary_hull(kind, a, b, eval_div, invalid);
    if (b.contains_pos_infinity() || b.contains_neg_infinity()) && a.has_finite_values() {
        result = join(&result, &FloatInterval::singleton(0.0));
    }
    result
}

pub fn compare(op: FloatCmpOp, a: &FloatInterval, b: &FloatInterval) -> Interval {
    if a.is_bottom() || b.is_bottom() {
        return Interval::empty();
    }

    let numeric = if a.has_numeric_values() && b.has_numeric_values() {
        numeric_compare(op, a, b)
    } else {
        Interval::empty()
    };
    let nan_possible = a.may_nan || b.may_nan;
    if !nan_possible {
        return numeric;
    }

    let nan_result = match op {
        FloatCmpOp::Ne => Interval::new(1, 1),
        FloatCmpOp::Lt | FloatCmpOp::Le | FloatCmpOp::Gt | FloatCmpOp::Ge | FloatCmpOp::Eq => {
            Interval::new(0, 0)
        }
    };
    join_integer(&numeric, &nan_result)
}

pub fn next_up(kind: FloatKind, value: f64) -> f64 {
    match kind {
        FloatKind::F32 => f64::from(next_up_f32(value as f32)),
        FloatKind::F64 => next_up_f64(value),
    }
}

pub fn next_down(kind: FloatKind, value: f64) -> f64 {
    match kind {
        FloatKind::F32 => f64::from(next_down_f32(value as f32)),
        FloatKind::F64 => next_down_f64(value),
    }
}

fn binary_hull(
    kind: FloatKind,
    a: &FloatInterval,
    b: &FloatInterval,
    operation: impl Fn(FloatKind, f64, f64) -> f64,
    invalid_non_nan_operands: bool,
) -> FloatInterval {
    if a.is_bottom() || b.is_bottom() {
        return FloatInterval::bottom();
    }

    let may_nan = a.may_nan || b.may_nan || invalid_non_nan_operands;
    if !a.has_numeric_values() || !b.has_numeric_values() {
        return FloatInterval::new(f64::INFINITY, f64::NEG_INFINITY, may_nan);
    }

    let mut low = f64::INFINITY;
    let mut high = f64::NEG_INFINITY;
    let mut found_numeric = false;
    for (left, right) in [
        (a.low, b.low),
        (a.low, b.high),
        (a.high, b.low),
        (a.high, b.high),
    ] {
        let value = operation(kind, left, right);
        if value.is_nan() {
            continue;
        }
        found_numeric = true;
        low = low.min(value);
        high = high.max(value);
    }

    if found_numeric {
        FloatInterval::new(low, high, may_nan)
    } else {
        FloatInterval::new(f64::INFINITY, f64::NEG_INFINITY, may_nan)
    }
}

fn eval_add(kind: FloatKind, left: f64, right: f64) -> f64 {
    match kind {
        FloatKind::F32 => f64::from((left as f32) + (right as f32)),
        FloatKind::F64 => left + right,
    }
}

fn eval_sub(kind: FloatKind, left: f64, right: f64) -> f64 {
    match kind {
        FloatKind::F32 => f64::from((left as f32) - (right as f32)),
        FloatKind::F64 => left - right,
    }
}

fn eval_mul(kind: FloatKind, left: f64, right: f64) -> f64 {
    match kind {
        FloatKind::F32 => f64::from((left as f32) * (right as f32)),
        FloatKind::F64 => left * right,
    }
}

fn eval_div(kind: FloatKind, left: f64, right: f64) -> f64 {
    match kind {
        FloatKind::F32 => f64::from((left as f32) / (right as f32)),
        FloatKind::F64 => left / right,
    }
}

fn numeric_compare(op: FloatCmpOp, a: &FloatInterval, b: &FloatInterval) -> Interval {
    match op {
        FloatCmpOp::Lt => {
            if a.high < b.low {
                Interval::new(1, 1)
            } else if a.low >= b.high {
                Interval::new(0, 0)
            } else {
                Interval::new(0, 1)
            }
        }
        FloatCmpOp::Le => {
            if a.high <= b.low {
                Interval::new(1, 1)
            } else if a.low > b.high {
                Interval::new(0, 0)
            } else {
                Interval::new(0, 1)
            }
        }
        FloatCmpOp::Gt => numeric_compare(FloatCmpOp::Lt, b, a),
        FloatCmpOp::Ge => numeric_compare(FloatCmpOp::Le, b, a),
        FloatCmpOp::Eq => {
            if a.low == a.high && b.low == b.high && a.low == b.low {
                Interval::new(1, 1)
            } else if a.high < b.low || a.low > b.high {
                Interval::new(0, 0)
            } else {
                Interval::new(0, 1)
            }
        }
        FloatCmpOp::Ne => {
            if a.high < b.low || a.low > b.high {
                Interval::new(1, 1)
            } else if a.low == a.high && b.low == b.high && a.low == b.low {
                Interval::new(0, 0)
            } else {
                Interval::new(0, 1)
            }
        }
    }
}

fn canonicalize_zero(value: f64) -> f64 {
    if value == 0.0 { 0.0 } else { value }
}

fn format_bound(value: f64) -> String {
    if value == f64::NEG_INFINITY {
        "-∞".to_string()
    } else if value == f64::INFINITY {
        "∞".to_string()
    } else {
        value.to_string()
    }
}

fn next_up_f32(value: f32) -> f32 {
    if value.is_nan() || value == f32::INFINITY {
        return value;
    }
    if value == 0.0 {
        return f32::from_bits(1);
    }
    let bits = value.to_bits();
    f32::from_bits(if value > 0.0 { bits + 1 } else { bits - 1 })
}

fn next_down_f32(value: f32) -> f32 {
    if value.is_nan() || value == f32::NEG_INFINITY {
        return value;
    }
    if value == 0.0 {
        return f32::from_bits((1u32 << 31) | 1);
    }
    let bits = value.to_bits();
    f32::from_bits(if value > 0.0 { bits - 1 } else { bits + 1 })
}

fn next_up_f64(value: f64) -> f64 {
    if value.is_nan() || value == f64::INFINITY {
        return value;
    }
    if value == 0.0 {
        return f64::from_bits(1);
    }
    let bits = value.to_bits();
    f64::from_bits(if value > 0.0 { bits + 1 } else { bits - 1 })
}

fn next_down_f64(value: f64) -> f64 {
    if value.is_nan() || value == f64::NEG_INFINITY {
        return value;
    }
    if value == 0.0 {
        return f64::from_bits((1u64 << 63) | 1);
    }
    let bits = value.to_bits();
    f64::from_bits(if value > 0.0 { bits - 1 } else { bits + 1 })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn join_keeps_nan_separate_from_numeric_bounds() {
        let value = FloatInterval::numeric(1.0, 2.0);
        assert_eq!(
            join(&value, &FloatInterval::nan_only()),
            FloatInterval::new(1.0, 2.0, true)
        );
    }

    #[test]
    fn comparisons_follow_nan_semantics() {
        let nan_or_one = FloatInterval::new(1.0, 1.0, true);
        let one = FloatInterval::singleton(1.0);

        assert_eq!(
            compare(FloatCmpOp::Eq, &nan_or_one, &one),
            Interval::new(0, 1)
        );
        assert_eq!(
            compare(FloatCmpOp::Ne, &nan_or_one, &one),
            Interval::new(0, 1)
        );
        assert_eq!(
            compare(FloatCmpOp::Lt, &FloatInterval::nan_only(), &one),
            Interval::new(0, 0)
        );
        assert_eq!(
            compare(FloatCmpOp::Ne, &FloatInterval::nan_only(), &one),
            Interval::new(1, 1)
        );
    }

    #[test]
    fn invalid_infinity_arithmetic_introduces_nan() {
        let result = add(
            FloatKind::F64,
            &FloatInterval::singleton(f64::INFINITY),
            &FloatInterval::singleton(f64::NEG_INFINITY),
        );
        assert_eq!(result, FloatInterval::nan_only());
    }

    #[test]
    fn zero_times_unbounded_interval_keeps_numeric_zero() {
        let result = mul(
            FloatKind::F64,
            &FloatInterval::singleton(0.0),
            &FloatInterval::numeric(f64::NEG_INFINITY, f64::INFINITY),
        );
        assert_eq!(result, FloatInterval::new(0.0, 0.0, true));
    }

    #[test]
    fn finite_value_divided_by_unbounded_infinity_keeps_zero() {
        let result = div(
            FloatKind::F64,
            &FloatInterval::numeric(f64::NEG_INFINITY, f64::INFINITY),
            &FloatInterval::singleton(f64::INFINITY),
        );
        assert_eq!(result, FloatInterval::new(0.0, 0.0, true));
    }

    #[test]
    fn division_by_interval_containing_zero_is_conservative() {
        let result = div(
            FloatKind::F32,
            &FloatInterval::singleton(1.0),
            &FloatInterval::numeric(-0.0, 0.0),
        );
        assert_eq!(result, FloatInterval::top());
    }

    #[test]
    fn next_values_respect_float_precision() {
        assert_eq!(next_up(FloatKind::F32, 0.0), f64::from(f32::from_bits(1)));
        assert_eq!(
            next_down(FloatKind::F64, 0.0),
            f64::from_bits((1u64 << 63) | 1)
        );
    }
}
