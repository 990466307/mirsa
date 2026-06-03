#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Interval {
    pub low: i128,  // 下界，None表示负无穷
    pub high: i128, // 上界，None表示正无穷
}

// 实现打印时间隔的表示形式，且max和min打印为∞
impl std::fmt::Display for Interval {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let low_str = if self.low == i128::MIN {
            "-∞".to_string()
        } else {
            self.low.to_string()
        };
        let high_str = if self.high == i128::MAX {
            "∞".to_string()
        } else {
            self.high.to_string()
        };
        if self.is_empty() {
            write!(f, "∅")
        } else {
            write!(f, "[{}, {}]", low_str, high_str)
        }
    }
}

impl Interval {
    pub fn new(low: i128, high: i128) -> Self {
        Interval { low, high }
    }

    // 空区间（Bottom）
    pub fn empty() -> Self {
        Interval { low: 1, high: 0 } // 约定：low > high 表示空
    }

    pub fn top() -> Self {
        Interval {
            low: i128::MIN,
            high: i128::MAX,
        }
    }

    pub fn is_empty(&self) -> bool {
        match (self.low, self.high) {
            (l, h) => l > h,
        }
    }

    pub fn get_interval(&self) -> (i128, i128) {
        (self.low, self.high)
    }
}

pub fn intersect(a: &Interval, b: &Interval) -> Interval {
    if a.is_empty() || b.is_empty() {
        return Interval::empty();
    }

    let new_low = a.low.max(b.low);
    let new_high = a.high.min(b.high);
    Interval::new(new_low, new_high)
}

pub fn widen(a: &Interval, b: &Interval) -> Interval {
    if a.is_empty() {
        return *b;
    }
    if b.is_empty() {
        return *a;
    }
    let new_low = if b.low < a.low { i128::MIN } else { a.low };
    let new_high = if b.high > a.high { i128::MAX } else { a.high };
    Interval::new(new_low, new_high)
}

pub fn join(a: &Interval, b: &Interval) -> Interval {
    if a.is_empty() {
        return *b;
    }
    if b.is_empty() {
        return *a;
    }

    let new_low = a.low.min(b.low);
    let new_high = a.high.max(b.high);

    Interval::new(new_low, new_high)
}

pub fn add(a: &Interval, b: &Interval) -> Interval {
    let new_low = safe_add(a.low, b.low);
    let new_high = safe_add(a.high, b.high);

    Interval::new(new_low, new_high)
}

pub fn sub(a: &Interval, b: &Interval) -> Interval {
    add(a, &neg(b))
}

pub fn neg(a: &Interval) -> Interval {
    let new_low = safe_neg(a.high);
    let new_high = safe_neg(a.low);

    Interval::new(new_low, new_high)
}

pub fn mul(a: &Interval, b: &Interval) -> Interval {
    let points = [
        (a.low, b.low),
        (a.low, b.high),
        (a.high, b.low),
        (a.high, b.high),
    ];

    let mut min_val = i128::MAX;
    let mut max_val = i128::MIN;

    for (x, y) in points.iter() {
        let product = safe_mul(*x, *y);
        min_val = min_val.min(product);
        max_val = max_val.max(product);
    }

    Interval::new(min_val, max_val)
}

pub fn div(a: &Interval, b: &Interval) -> Interval {
    if b.low <= 0 && b.high >= 0 {
        return Interval::top();
    }

    let min_val;
    let max_val;

    match (a.low > 0, b.low > 0) {
        (true, true) => {
            if a.high == i128::MAX && b.high == i128::MAX {
                min_val = 0;
                max_val = i128::MAX;
            } else if a.high == i128::MAX {
                min_val = safe_div(a.low, b.high);
                max_val = i128::MAX;
            } else if b.high == i128::MAX {
                min_val = 0;
                max_val = safe_div(a.high, b.low);
            } else {
                min_val = safe_div(a.low, b.high);
                max_val = safe_div(a.high, b.low);
            }
        }
        (true, false) => {
            if a.high == i128::MAX && b.low == i128::MIN {
                min_val = i128::MIN;
                max_val = 0;
            } else if a.high == i128::MAX {
                min_val = i128::MIN;
                max_val = safe_div(a.low, b.low);
            } else if b.low == i128::MIN {
                min_val = safe_div(a.high, b.high);
                max_val = 0;
            } else {
                min_val = safe_div(a.high, b.high);
                max_val = safe_div(a.low, b.low);
            }
        }
        (false, true) => {
            if a.high < 0 {
                if a.low == i128::MIN && b.high == i128::MAX {
                    min_val = i128::MIN;
                    max_val = 0;
                } else if a.low == i128::MIN {
                    min_val = i128::MIN;
                    max_val = safe_div(a.high, b.high);
                } else if b.high == i128::MAX {
                    min_val = safe_div(a.low, b.low);
                    max_val = 0;
                } else {
                    min_val = safe_div(a.low, b.low);
                    max_val = safe_div(a.high, b.high);
                }
            } else {
                if a.low == i128::MIN {
                    min_val = i128::MIN;
                    max_val = safe_div(a.high, b.low);
                } else {
                    min_val = safe_div(a.low, b.low);
                    max_val = safe_div(a.high, b.low);
                }
            }
        }
        (false, false) => {
            if a.high < 0 {
                if a.low == i128::MIN && b.low == i128::MIN {
                    min_val = 0;
                    max_val = i128::MAX;
                } else if a.low == i128::MIN {
                    min_val = safe_div(a.high, b.low);
                    max_val = i128::MAX;
                } else if b.low == i128::MIN {
                    min_val = 0;
                    max_val = safe_div(a.low, b.high);
                } else {
                    min_val = safe_div(a.high, b.low);
                    max_val = safe_div(a.low, b.high);
                }
            } else {
                if a.low == i128::MIN {
                    min_val = safe_div(a.high, b.high);
                    max_val = i128::MAX;
                } else {
                    min_val = safe_div(a.high, b.high);
                    max_val = safe_div(a.low, b.high);
                }
            }
        }
    };
    Interval::new(min_val, max_val)
}

pub fn lt(a: &Interval, b: &Interval) -> Interval {
    if a.is_empty() || b.is_empty() {
        return Interval::empty();
    }
    if a.high < b.low {
        Interval::new(1, 1) // true
    } else if a.low >= b.high {
        Interval::new(0, 0) // false
    } else {
        Interval::new(0, 1) // unknown
    }
}

pub fn le(a: &Interval, b: &Interval) -> Interval {
    if a.is_empty() || b.is_empty() {
        return Interval::empty();
    }
    if a.high <= b.low {
        Interval::new(1, 1) // true
    } else if a.low > b.high {
        Interval::new(0, 0) // false
    } else {
        Interval::new(0, 1) // unknown
    }
}

pub fn gt(a: &Interval, b: &Interval) -> Interval {
    if a.is_empty() || b.is_empty() {
        return Interval::empty();
    }
    if a.low > b.high {
        Interval::new(1, 1) // true
    } else if a.high <= b.low {
        Interval::new(0, 0) // false
    } else {
        Interval::new(0, 1) // unknown
    }
}

pub fn ge(a: &Interval, b: &Interval) -> Interval {
    if a.is_empty() || b.is_empty() {
        return Interval::empty();
    }
    if a.low >= b.high {
        Interval::new(1, 1) // true
    } else if a.high < b.low {
        Interval::new(0, 0) // false
    } else {
        Interval::new(0, 1) // unknown
    }
}

pub fn neq(a: &Interval, b: &Interval) -> Interval {
    if a.is_empty() || b.is_empty() {
        return Interval::empty();
    }
    if a.high < b.low || a.low > b.high {
        Interval::new(1, 1) // true
    } else if a.low == a.high && b.low == b.high && a.low == b.low {
        Interval::new(0, 0) // false
    } else {
        Interval::new(0, 1) // unknown
    }
}

pub fn eq(a: &Interval, b: &Interval) -> Interval {
    if a.is_empty() || b.is_empty() {
        return Interval::empty();
    }
    if a.low == a.high && b.low == b.high && a.low == b.low {
        Interval::new(1, 1) // true
    } else if a.high < b.low || a.low > b.high {
        Interval::new(0, 0) // false
    } else {
        Interval::new(0, 1) // unknown
    }
}

pub fn bitand(a: &Interval, b: &Interval) -> Interval {
    if a.is_empty() || b.is_empty() {
        return Interval::empty();
    }
    if a.low == a.high && b.low == b.high {
        return Interval::new(a.low & b.low, a.low & b.low);
    }
    Interval::top()
}

pub fn bitor(a: &Interval, b: &Interval) -> Interval {
    if a.is_empty() || b.is_empty() {
        return Interval::empty();
    }
    if a.low == a.high && b.low == b.high {
        return Interval::new(a.low | b.low, a.low | b.low);
    }
    Interval::top()
}

pub fn bitxor(a: &Interval, b: &Interval) -> Interval {
    if a.is_empty() || b.is_empty() {
        return Interval::empty();
    }
    if a.low == a.high && b.low == b.high {
        return Interval::new(a.low ^ b.low, a.low ^ b.low);
    }
    Interval::top()
}

#[cfg(test)]
mod tests {
    use super::{Interval, eq, ge, gt, le, lt, neq};

    #[test]
    fn comparisons_propagate_empty_intervals() {
        let empty = Interval::empty();
        let value = Interval::new(128, 128);

        for result in [
            lt(&empty, &value),
            le(&empty, &value),
            gt(&empty, &value),
            ge(&empty, &value),
            eq(&empty, &value),
            neq(&empty, &value),
            lt(&value, &empty),
            le(&value, &empty),
            gt(&value, &empty),
            ge(&value, &empty),
            eq(&value, &empty),
            neq(&value, &empty),
        ] {
            assert_eq!(result, empty);
        }
    }
}

fn safe_div(a: i128, b: i128) -> i128 {
    if a == i128::MIN && b < 0 {
        i128::MAX
    } else if a == i128::MIN && b > 0 {
        i128::MIN
    } else if a == i128::MAX && b > 0 {
        i128::MAX
    } else if a == i128::MAX && b < 0 {
        i128::MIN
    } else {
        a / b
    }
}

fn safe_add(a: i128, b: i128) -> i128 {
    if a == i128::MAX || b == i128::MAX {
        i128::MAX
    } else if a == i128::MIN || b == i128::MIN {
        i128::MIN
    } else {
        a.saturating_add(b)
    }
}

fn safe_neg(a: i128) -> i128 {
    if a == i128::MIN {
        i128::MAX
    } else if a == i128::MAX {
        i128::MIN
    } else {
        a.saturating_neg()
    }
}

fn safe_mul(a: i128, b: i128) -> i128 {
    if a == 0 || b == 0 {
        0
    } else if (a == i128::MAX && b > 0) || (b == i128::MAX && a > 0) {
        i128::MAX
    } else if (a == i128::MIN && b < 0) || (b == i128::MIN && a < 0) {
        i128::MAX
    } else if (a == i128::MAX && b < 0) || (b == i128::MAX && a < 0) {
        i128::MIN
    } else if (a == i128::MIN && b > 0) || (b == i128::MIN && a > 0) {
        i128::MIN
    } else {
        a.saturating_mul(b)
    }
}
