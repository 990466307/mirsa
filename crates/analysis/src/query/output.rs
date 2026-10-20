use super::bind::Binder;
use super::*;
use mirsa_dsl::RequestKind;
use serde::Serialize;
use std::fmt;

#[derive(Clone, Serialize)]
pub struct Report {
    #[serde(skip)]
    location: String,
    pub output: Output,
}
impl fmt::Debug for Report {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Report")
            .field("output", &self.output)
            .finish()
    }
}

impl fmt::Display for Report {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "at '{}:", self.location)?;
        match &self.output {
            Output::Facts(facts) => write!(f, "{facts}"),
            Output::Check(check) => write!(f, "{}", check.result),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "kind", content = "result", rename_all = "snake_case")]
pub enum Output {
    Check(CheckReport),
    Facts(FactOutput),
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum FactValue {
    Bits {
        bits: String,
        width: u64,
    },
    Allocation {
        alignment: u64,
        origin: String,
        extent_low: String,
        extent_high: String,
        may_be_absent: bool,
        may_be_live: bool,
        may_be_dead: bool,
        may_be_zero: bool,
        may_be_one: bool,
        may_be_many: bool,
    },
    Unreachable,
    Integer {
        low: String,
        high: String,
    },
    Float {
        low: String,
        high: String,
        may_nan: bool,
    },
    Bool {
        value: Option<bool>,
    },
    Pointer {
        may_be_null: bool,
        may_be_invalid: bool,
        unknown_target: bool,
        targets: Vec<PointerTarget>,
    },
    Unknown {
        reason: UnknownReason,
    },
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct PointerTarget {
    pub allocation: String,
    pub offset_low: String,
    pub offset_high: String,
    pub extent_low: Option<String>,
    pub extent_high: Option<String>,
    pub status: Option<String>,
}

impl<'a, 'tcx> AnalysisView<'a, 'tcx> {
    pub fn check(
        &self,
        expression: &str,
        location: Location,
        bindings: &Bindings<'tcx>,
    ) -> Result<Proof, Vec<Diagnostic>> {
        let syntax = parse_expression(expression)?;
        let binder = Binder {
            tcx: self.tcx,
            body: self.body,
            bindings,
        };
        let expr = binder
            .bind(&syntax, Some(value::ValueType::Rust(self.tcx.types.bool)))
            .map_err(|error| vec![error])?;
        let prepared = self
            .prepare(&[location], &[&expr], false)
            .map_err(|error| {
                vec![Diagnostic {
                    span: syntax.span.clone(),
                    message: format!("cannot query location: {error:?}"),
                }]
            })?;
        let mut view = prepared
            .as_ref()
            .unwrap_or(self)
            .at(location)
            .map_err(|error| {
                vec![Diagnostic {
                    span: syntax.span,
                    message: format!("cannot query location: {error:?}"),
                }]
            })?;
        Ok(view.prove(&Predicate::Value(expr)))
    }

    pub fn query(
        &self,
        source: &str,
        bindings: &Bindings<'tcx>,
    ) -> Result<Report, Vec<Diagnostic>> {
        self.execute(&parse(source)?, bindings)
    }
    pub fn execute(
        &self,
        document: &Document,
        bindings: &Bindings<'tcx>,
    ) -> Result<Report, Vec<Diagnostic>> {
        let statement = document.statement();
        let scoped = bindings.for_location(&statement.location);
        let binder = Binder {
            tcx: self.tcx,
            body: self.body,
            bindings: &scoped,
        };
        let location = bindings
            .locations
            .get(&statement.location)
            .copied()
            .ok_or_else(|| {
                vec![Diagnostic {
                    span: statement.span.clone(),
                    message: format!("unknown location binding '{}", statement.location),
                }]
            })?;
        let expected = (statement.kind == RequestKind::Check)
            .then_some(value::ValueType::Rust(self.tcx.types.bool));
        let expression = binder
            .bind(&statement.expression, expected)
            .map_err(|error| vec![error])?;
        let prepared = self
            .prepare(
                &[location],
                &[&expression],
                statement.kind == RequestKind::Facts,
            )
            .map_err(|error| {
                vec![Diagnostic {
                    span: statement.span.clone(),
                    message: format!("cannot query location: {error:?}"),
                }]
            })?;
        let mut view = prepared
            .as_ref()
            .unwrap_or(self)
            .at(location)
            .map_err(|error| {
                vec![Diagnostic {
                    span: statement.span.clone(),
                    message: format!("cannot query location: {error:?}"),
                }]
            })?;
        let output = match statement.kind {
            RequestKind::Check => {
                let property = Predicate::Value(expression);
                Output::Check(CheckReport::new(view.prove(&property)))
            }
            RequestKind::Facts => {
                Output::Facts(view.facts(&expression, &statement.expression, &scoped)?)
            }
        };
        Ok(Report {
            location: statement.location.clone(),
            output,
        })
    }
}
