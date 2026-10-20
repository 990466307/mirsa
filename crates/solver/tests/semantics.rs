#[cfg(not(feature = "z3"))]
#[test]
fn disabled_backend_stays_unknown() {
    use mirsa_solver::{Formula, Problem, UnknownReason, Verdict, solve};
    let result = solve(
        &Problem {
            assumptions: vec![],
            property: Formula::Literal(true),
        },
        std::time::Duration::from_secs(1),
    );
    assert_eq!(result.verdict, Verdict::Unknown);
    assert_eq!(result.reason, Some(UnknownReason::BackendDisabled));
}

#[cfg(feature = "z3")]
mod tests {
    use mirsa_solver::*;
    use std::time::Duration;
    fn verdict(a: Vec<Formula>, p: Formula) -> Verdict {
        solve(
            &Problem {
                assumptions: a,
                property: p,
            },
            Duration::from_secs(2),
        )
        .verdict
    }
    fn u8t(n: u128) -> Term {
        Term::Constant(
            n,
            ScalarType {
                bits: 8,
                signed: false,
            },
        )
    }
    fn cmp(a: Term, b: Term) -> Formula {
        Formula::Compare(Comparison::Eq, a, b)
    }
    fn fp(x: f64) -> FloatTerm {
        FloatTerm::FromBits(Term::Constant(
            x.to_bits() as u128,
            ScalarType {
                bits: 64,
                signed: false,
            },
        ))
    }
    #[test]
    fn verdicts() {
        assert_eq!(verdict(vec![], Formula::Literal(true)), Verdict::Proven);
        assert_eq!(verdict(vec![], Formula::Literal(false)), Verdict::Refuted);
        assert_eq!(
            verdict(vec![Formula::Literal(false)], Formula::Literal(true)),
            Verdict::Unreachable
        );
        assert_eq!(
            verdict(
                vec![],
                cmp(
                    Term::Variable(
                        0,
                        ScalarType {
                            bits: 8,
                            signed: false
                        }
                    ),
                    u8t(1)
                )
            ),
            Verdict::Unknown
        );
    }
    #[test]
    fn wrapping_and_overflow() {
        let a = Term::Binary(BinaryOp::Add, Box::new(u8t(255)), Box::new(u8t(1)));
        assert_eq!(verdict(vec![], cmp(a, u8t(0))), Verdict::Proven);
        let ov = Term::Overflow(BinaryOp::Add, Box::new(u8t(255)), Box::new(u8t(1)));
        assert_eq!(
            verdict(vec![], cmp(ov, Term::Constant(1, ScalarType::BOOL))),
            Verdict::Proven
        );
    }
    #[test]
    fn integer_encoding_matches_unsigned_bit_arithmetic() {
        // MathCompare selects the integer backend while the operands retain
        // machine-integer wrapping and division-by-zero semantics.
        for (op, a, b, expected) in [
            (BinaryOp::Div, 255, 2, 127),
            (BinaryOp::Rem, 255, 2, 1),
            (BinaryOp::Div, 17, 0, 255),
            (BinaryOp::Rem, 17, 0, 17),
            (BinaryOp::Div, 256, 2, 0),
        ] {
            let value = Term::Binary(op, Box::new(u8t(a)), Box::new(u8t(b)));
            assert_eq!(
                verdict(
                    vec![],
                    Formula::MathCompare(
                        Comparison::Eq,
                        IntTerm::FromBits(value),
                        IntTerm::Constant(expected.to_string())
                    )
                ),
                Verdict::Proven
            );
        }
    }
    #[test]
    fn mathematical_division_truncates_toward_zero() {
        let d = IntTerm::Binary(
            BinaryOp::Div,
            Box::new(IntTerm::Constant("-7".into())),
            Box::new(IntTerm::Constant("3".into())),
        );
        assert_eq!(
            verdict(
                vec![],
                Formula::MathCompare(Comparison::Eq, d, IntTerm::Constant("-2".into()))
            ),
            Verdict::Proven
        );
    }
    #[test]
    fn float_nan_zero_remainder_and_cast() {
        assert_eq!(
            verdict(
                vec![],
                Formula::FloatCompare(Comparison::Eq, fp(f64::NAN), fp(f64::NAN))
            ),
            Verdict::Refuted
        );
        assert_eq!(
            verdict(
                vec![],
                Formula::FloatCompare(Comparison::Eq, fp(0.0), fp(-0.0))
            ),
            Verdict::Proven
        );
        let rem = FloatTerm::Binary(BinaryOp::Rem, Box::new(fp(-7.0)), Box::new(fp(3.0)));
        assert_eq!(
            verdict(vec![], Formula::FloatCompare(Comparison::Eq, rem, fp(-1.0))),
            Verdict::Proven
        );
        assert_eq!(
            verdict(
                vec![],
                cmp(
                    Term::FloatToInteger(
                        Box::new(fp(f64::INFINITY)),
                        ScalarType {
                            bits: 8,
                            signed: false
                        }
                    ),
                    u8t(255)
                )
            ),
            Verdict::Proven
        );
    }
    #[test]
    fn timeout_stays_unknown() {
        assert_eq!(
            solve(
                &Problem {
                    assumptions: vec![],
                    property: Formula::Literal(true)
                },
                Duration::ZERO
            )
            .verdict,
            Verdict::Unknown
        );
    }
}
