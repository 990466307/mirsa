#[cfg(test)]
mod tests {
    #[test]
    fn parser_accepts_requests_and_rejects_bad_input() {
        for source in [
            "!is_null($x) at 'p",
            "math($a) + 1 > 0 at 'p",
            "facts($x) at 'p",
            "facts(same_allocation($x, $y)) at 'p",
        ] {
            assert!(mirsa_dsl::parse(source).is_ok(), "{source}");
            assert!(
                mirsa_dsl::parse(&format!("{source};")).is_err(),
                "{source};"
            );
        }
        for s in [
            "",
            "// comment only",
            "true at 'p; false at 'p;",
            "true at 'p; facts($x) at 'p;",
            "old_name = true at 'p",
            "is_null() at 'p",
            "1 < 2 < 3 at 'p",
            "1wat at 'p",
            "true at 'p; junk",
            "true at 'p junk",
            "true at 'p false at 'p",
            "true at 'p;;",
            "facts() at 'p",
            "facts($x, $a) at 'p",
            "facts($x,) at 'p",
            "facts($x) == true at 'p",
            "check n at 'p = true;",
            "oracle n at 'p = true;",
            "facts n at 'p = [$x];",
        ] {
            assert!(mirsa_dsl::parse(s).is_err(), "{s}");
        }
    }

    #[test]
    fn requests_preserve_source_spans() {
        for request in ["$i < len($s)", "facts($s)"] {
            let statement = format!("{request} at 'point");
            let source = format!("// 中文\n{statement} // trailing comment\n");
            let doc = mirsa_dsl::parse(&source).unwrap();
            assert_eq!(&source[doc.statement().span.clone()], statement);
            assert_eq!(doc.statement().location, "point");
            let expression = request
                .strip_prefix("facts(")
                .map_or(request, |s| s.strip_suffix(')').unwrap());
            assert_eq!(&source[doc.statement().expression.span.clone()], expression);
        }
    }
    #[test]
    fn parser_precedence() {
        use mirsa_dsl::{Binary, ExprKind};
        let d = mirsa_dsl::parse("facts(1 + 2 * 3) at 'p").unwrap();
        let ExprKind::Binary(Binary::Add, _, rhs) = &d.statement().expression.kind else {
            panic!("addition expected")
        };
        assert!(matches!(rhs.kind, ExprKind::Binary(Binary::Mul, _, _)));
    }

    #[test]
    fn standalone_expression_and_source_spans() {
        use mirsa_dsl::{ExprKind, parse, parse_expression};
        let source = "// 中文\n!is_null($p) && math($n) >= 0";
        let expr = parse_expression(source).unwrap();
        assert_eq!(&source[expr.span.clone()], "!is_null($p) && math($n) >= 0");
        let ExprKind::Binary(_, left, right) = &expr.kind else {
            panic!()
        };
        assert_eq!(&source[left.span.clone()], "!is_null($p)");
        assert_eq!(&source[right.span.clone()], "math($n) >= 0");
        let source = "facts(len($s)) at 'call";
        let doc = parse(source).unwrap();
        let statement = doc.statement();
        assert_eq!(statement.location, "call");
        assert_eq!(&source[statement.expression.span.clone()], "len($s)");
        for source in [
            "",
            "facts($p)",
            "true;",
            "true at 'p",
            "x = true",
            "true; false at 'p;",
        ] {
            assert!(parse_expression(source).is_err(), "{source}");
        }
    }
}
