use crate::*;
use chumsky::prelude::*;

#[derive(Clone, Debug, PartialEq)]
enum Token {
    Name(String),
    Number(String),
    Symbol(String),
}

type Error<'a> = extra::Err<Rich<'a, Token>>;

fn valid_number(text: &str) -> bool {
    if text.contains('.') {
        return text.ends_with("f32") || text.ends_with("f64");
    }
    let (digits, hex) = text.strip_prefix("0x").map_or((text, false), |s| (s, true));
    let count = digits
        .bytes()
        .take_while(|b| {
            if hex {
                b.is_ascii_hexdigit()
            } else {
                b.is_ascii_digit()
            }
        })
        .count();
    let suffix = &digits[count..];
    count > 0
        && (suffix.is_empty()
            || ((suffix.starts_with('i') || suffix.starts_with('u')) && is_scalar(suffix)))
}

fn expression<'a>() -> impl Parser<'a, &'a [Token], Expr, Error<'a>> + Clone {
    let symbol = |s: &str| just(Token::Symbol(s.to_owned()));
    let keyword = |s: &str| just(Token::Name(s.to_owned()));
    let name = select! { Token::Name(n) if is_name(&n) => n };
    let scalar = select! { Token::Name(n) if is_scalar(&n) => n };
    let ty = symbol("$")
        .ignore_then(name.clone())
        .map(|name| TypeRef {
            name,
            binding: true,
        })
        .or(scalar.clone().map(|name| TypeRef {
            name,
            binding: false,
        }));
    recursive(|expr| {
        let builtin = select! { Token::Name(n) if Builtin::named(&n).is_some() => Builtin::named(&n).unwrap() }
            .then(expr.clone().separated_by(symbol(",")).collect::<Vec<_>>().delimited_by(symbol("("), symbol(")")))
            .try_map(|(op, args), span| if args.len() == op.arity() { Ok(ExprKind::Builtin(op, args)) } else { Err(Rich::custom(span, "wrong builtin argument count")) });
        let layout = keyword("size_of")
            .to(false)
            .or(keyword("align_of").to(true))
            .then_ignore(symbol("::"))
            .then(ty.clone().delimited_by(symbol("<"), symbol(">")))
            .then_ignore(symbol("("))
            .then_ignore(symbol(")"))
            .map(|(align, ty)| ExprKind::TypeLayout { align, ty });
        let atom = choice((
            symbol("$").ignore_then(name.clone()).map(ExprKind::Binding),
            select! { Token::Number(n) => ExprKind::Number(n) },
            keyword("true").to(ExprKind::Bool(true)),
            keyword("false").to(ExprKind::Bool(false)),
            builtin,
            layout,
        ))
        .map_with(|kind, e| Expr {
            kind,
            span: {
                let s: SimpleSpan = e.span();
                s.into_range()
            },
        })
        .or(expr.clone().delimited_by(symbol("("), symbol(")")));
        #[derive(Clone)]
        enum Suffix {
            Field(String),
            Index(Expr),
        }
        let suffix = symbol(".")
            .ignore_then(
                name.clone()
                    .or(select! { Token::Number(n) if n.bytes().all(|b| b.is_ascii_digit()) => n }),
            )
            .map(Suffix::Field)
            .or(expr
                .clone()
                .delimited_by(symbol("["), symbol("]"))
                .map(Suffix::Index));
        let postfix = atom.foldl_with(suffix.repeated(), |left, suffix, e| Expr {
            kind: match suffix {
                Suffix::Field(n) => ExprKind::Field(Box::new(left), n),
                Suffix::Index(i) => ExprKind::Index(Box::new(left), Box::new(i)),
            },
            span: {
                let s: SimpleSpan = e.span();
                s.into_range()
            },
        });
        let unary = choice((
            symbol("!").to(Unary::Not),
            symbol("-").to(Unary::Neg),
            symbol("*").to(Unary::Deref),
        ))
        .map_with(|op, e| {
            (op, {
                let s: SimpleSpan = e.span();
                s.start
            })
        })
        .repeated()
        .collect::<Vec<_>>()
        .then(postfix)
        .map(|(ops, value)| {
            ops.into_iter()
                .rev()
                .fold(value, |value, (op, start)| Expr {
                    span: start..value.span.end,
                    kind: ExprKind::Unary(op, Box::new(value)),
                })
        });
        let cast = unary
            .foldl_with(
                keyword("as").ignore_then(scalar.clone()).repeated(),
                |left, ty, e| Expr {
                    kind: ExprKind::Cast(Box::new(left), ty),
                    span: {
                        let s: SimpleSpan = e.span();
                        s.into_range()
                    },
                },
            )
            .boxed();
        let levels: &[&[(&str, Binary)]] = &[
            &[("*", Binary::Mul), ("/", Binary::Div), ("%", Binary::Rem)],
            &[("+", Binary::Add), ("-", Binary::Sub)],
            &[("<<", Binary::Shl), (">>", Binary::Shr)],
            &[("&", Binary::BitAnd)],
            &[("^", Binary::BitXor)],
            &[("|", Binary::BitOr)],
        ];
        let mut current = cast;
        for level in levels {
            let mut op = symbol(level[0].0).to(level[0].1).boxed();
            for (s, b) in &level[1..] {
                op = op.or(symbol(s).to(*b)).boxed();
            }
            current = current
                .clone()
                .foldl_with(op.then(current).repeated(), binary)
                .boxed();
        }
        let compare = choice((
            symbol("==").to(Binary::Eq),
            symbol("!=").to(Binary::Ne),
            symbol("<").to(Binary::Lt),
            symbol("<=").to(Binary::Le),
            symbol(">").to(Binary::Gt),
            symbol(">=").to(Binary::Ge),
        ));
        let compare = current
            .clone()
            .then(compare.then(current).or_not())
            .map(|(a, tail)| match tail {
                Some((op, b)) => {
                    let span = a.span.start..b.span.end;
                    Expr {
                        kind: ExprKind::Binary(op, Box::new(a), Box::new(b)),
                        span,
                    }
                }
                None => a,
            });
        let and = compare.clone().foldl_with(
            symbol("&&").to(Binary::And).then(compare).repeated(),
            binary,
        );
        let or = and
            .clone()
            .foldl_with(symbol("||").to(Binary::Or).then(and).repeated(), binary);
        or.clone()
            .separated_by(symbol("=>"))
            .at_least(1)
            .collect::<Vec<_>>()
            .map(|mut values| {
                let mut result = values.pop().unwrap();
                while let Some(left) = values.pop() {
                    let span = left.span.start..result.span.end;
                    result = Expr {
                        span,
                        kind: ExprKind::Binary(Binary::Implies, Box::new(left), Box::new(result)),
                    };
                }
                result
            })
    })
}

fn grammar<'a>() -> impl Parser<'a, &'a [Token], Statement, Error<'a>> {
    let symbol = |s: &str| just(Token::Symbol(s.to_owned()));
    let keyword = |s: &str| just(Token::Name(s.to_owned()));
    let name = select! { Token::Name(n) if is_name(&n) => n };
    let expr = expression();
    let request = keyword("facts")
        .ignore_then(expr.clone().delimited_by(symbol("("), symbol(")")))
        .map(|expression| (RequestKind::Facts, expression))
        .or(expr.map(|expression| (RequestKind::Check, expression)));
    request
        .then_ignore(keyword("at"))
        .then(symbol("'").ignore_then(name))
        .map_with(|((kind, expression), location), e| Statement {
            kind,
            location,
            expression,
            span: e.span().into_range(),
        })
        .then_ignore(end())
}

fn binary<'a>(
    left: Expr,
    (op, right): (Binary, Expr),
    e: &mut chumsky::input::MapExtra<'a, '_, &'a [Token], Error<'a>>,
) -> Expr {
    Expr {
        kind: ExprKind::Binary(op, Box::new(left), Box::new(right)),
        span: {
            let s: SimpleSpan = e.span();
            s.into_range()
        },
    }
}

fn tokenize(source: &str) -> Result<Tokens, Vec<Diagnostic>> {
    if source.len() > 65536 {
        return Err(vec![Diagnostic {
            span: 0..source.len(),
            message: "document exceeds 64 KiB".into(),
        }]);
    }
    let digits = one_of::<_, &str, extra::Err<Rich<char>>>('0'..='9')
        .repeated()
        .at_least(1);
    let suffix = one_of('a'..='z')
        .then(one_of('a'..='z').or(one_of('0'..='9')).repeated())
        .or_not();
    let hex = just("0x")
        .then(one_of("0123456789abcdefABCDEF").repeated().at_least(1))
        .then(suffix.clone())
        .to_slice();
    let float = digits
        .clone()
        .then(just('.'))
        .then(digits.clone())
        .then(
            one_of("eE")
                .then(one_of("+-").or_not())
                .then(digits.clone())
                .or_not(),
        )
        .then(choice((just("f32"), just("f64"))))
        .to_slice();
    let integer = digits.then(suffix).to_slice();
    let number = choice((hex, float, integer)).try_map(|s: &str, span| {
        if valid_number(s) {
            Ok(Token::Number(s.to_owned()))
        } else {
            Err(Rich::custom(span, "invalid numeric literal or suffix"))
        }
    });
    let ident = one_of('a'..='z')
        .or(one_of('A'..='Z'))
        .or(just('_'))
        .then(
            one_of('a'..='z')
                .or(one_of('A'..='Z'))
                .or(just('_'))
                .or(one_of('0'..='9'))
                .repeated(),
        )
        .to_slice()
        .map(|s: &str| Token::Name(s.to_owned()));
    let symbols = choice((
        just("::"),
        just("=>"),
        just("||"),
        just("&&"),
        just("=="),
        just("!="),
        just("<="),
        just(">="),
        just("<<"),
        just(">>"),
    ))
    .map(|s: &str| Token::Symbol(s.to_owned()))
    .or(one_of("$'()[];,.=<>|^&+-*/%!{}").map(|c: char| Token::Symbol(c.to_string())));
    let comment = just("//")
        .then(any().and_is(just('\n').not()).repeated())
        .ignored();
    let padding = one_of(" \t\r\n").ignored().or(comment).repeated();
    let lexer = choice((number, ident, symbols))
        .map_with(|t, e| {
            (t, {
                let s: SimpleSpan = e.span();
                s.into_range()
            })
        })
        .padded_by(padding)
        .repeated()
        .collect::<Vec<_>>()
        .then_ignore(end());
    let tokens = lexer
        .parse(source)
        .into_result()
        .map_err(|errors: Vec<Rich<char>>| {
            errors
                .into_iter()
                .map(|e| Diagnostic {
                    span: e.span().into_range(),
                    message: e.to_string(),
                })
                .collect::<Vec<_>>()
        })?;
    if tokens.len() > 8192 {
        return Err(vec![Diagnostic {
            span: 0..source.len(),
            message: "too many tokens".into(),
        }]);
    }
    let mut depth = 0usize;
    for (token, span) in &tokens {
        match token {
            Token::Symbol(s) if matches!(s.as_str(), "(" | "[") => {
                depth += 1;
                if depth > 64 {
                    return Err(vec![Diagnostic {
                        span: span.clone(),
                        message: "nesting exceeds 64".into(),
                    }]);
                }
            }
            Token::Symbol(s) if matches!(s.as_str(), ")" | "]") => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    Ok(Tokens {
        raw: tokens.iter().map(|(token, _)| token.clone()).collect(),
        spans: tokens.into_iter().map(|(_, span)| span).collect(),
        source_len: source.len(),
    })
}

struct Tokens {
    raw: Vec<Token>,
    spans: Vec<Span>,
    source_len: usize,
}

impl Tokens {
    fn byte_span(&self, span: Span) -> Span {
        self.spans
            .get(span.start)
            .map_or(self.source_len, |s| s.start)
            ..span
                .end
                .checked_sub(1)
                .and_then(|i| self.spans.get(i))
                .map_or(self.source_len, |s| s.end)
    }

    fn diagnostics(&self, errors: Vec<Rich<'_, Token>>) -> Vec<Diagnostic> {
        errors
            .into_iter()
            .map(|e| Diagnostic {
                span: self.byte_span(e.span().into_range()),
                message: format!("invalid DSL syntax: {:?}", e.reason()),
            })
            .collect()
    }

    fn expression_spans(&self, expr: &mut Expr) -> Result<(), Vec<Diagnostic>> {
        if spans(expr, &|s| self.byte_span(s), 0) {
            Ok(())
        } else {
            Err(vec![Diagnostic {
                span: expr.span.clone(),
                message: "expression depth exceeds 64".into(),
            }])
        }
    }
}

fn spans(expr: &mut Expr, convert: &impl Fn(Span) -> Span, depth: usize) -> bool {
    if depth > 64 {
        return false;
    }
    expr.span = convert(expr.span.clone());
    match &mut expr.kind {
        ExprKind::Unary(_, v) | ExprKind::Cast(v, _) | ExprKind::Field(v, _) => {
            spans(v, convert, depth + 1)
        }
        ExprKind::Binary(_, a, b) | ExprKind::Index(a, b) => {
            spans(a, convert, depth + 1) && spans(b, convert, depth + 1)
        }
        ExprKind::Builtin(_, args) => args.iter_mut().all(|v| spans(v, convert, depth + 1)),
        _ => true,
    }
}

pub fn parse(source: &str) -> Result<Document, Vec<Diagnostic>> {
    let tokens = tokenize(source)?;
    let mut statement = grammar()
        .parse(tokens.raw.as_slice())
        .into_result()
        .map_err(|errors| tokens.diagnostics(errors))?;
    statement.span = tokens.byte_span(statement.span.clone());
    tokens.expression_spans(&mut statement.expression)?;
    Ok(Document {
        source: source.to_owned(),
        statement,
    })
}

pub fn parse_expression(source: &str) -> Result<Expr, Vec<Diagnostic>> {
    let tokens = tokenize(source)?;
    let mut expr = expression()
        .then_ignore(end())
        .parse(tokens.raw.as_slice())
        .into_result()
        .map_err(|errors| tokens.diagnostics(errors))?;
    tokens.expression_spans(&mut expr)?;
    Ok(expr)
}
