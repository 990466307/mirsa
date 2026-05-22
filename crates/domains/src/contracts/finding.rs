use rustc_middle::ty::TyCtxt;
use rustc_span::Span;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Level {
    Safe,
    Possible,
    Definite,
}

impl Level {
    pub(crate) fn combine(self, other: Level) -> Level {
        match (self, other) {
            (Level::Definite, _) | (_, Level::Definite) => Level::Definite,
            (Level::Possible, _) | (_, Level::Possible) => Level::Possible,
            (Level::Safe, Level::Safe) => Level::Safe,
        }
    }

    pub(crate) fn is_safe(self) -> bool {
        self == Level::Safe
    }

    pub(crate) fn code(self, definite: &'static str, possible: &'static str) -> &'static str {
        match self {
            Level::Definite => definite,
            Level::Possible => possible,
            Level::Safe => unreachable!(),
        }
    }
}

pub(crate) struct Finding {
    pub(crate) span: Span,
    pub(crate) code: &'static str,
    pub(crate) message: String,
    pub(crate) notes: Vec<String>,
}

impl Finding {
    pub(crate) fn new(
        span: Span,
        code: &'static str,
        message: impl Into<String>,
        notes: Vec<String>,
    ) -> Self {
        Self {
            span,
            code,
            message: message.into(),
            notes,
        }
    }

    pub(crate) fn for_level(
        level: Level,
        span: Span,
        definite_code: &'static str,
        possible_code: &'static str,
        definite_message: impl Into<String>,
        possible_message: impl Into<String>,
        notes: Vec<String>,
    ) -> Option<Self> {
        if level.is_safe() {
            return None;
        }

        let message = match level {
            Level::Definite => definite_message.into(),
            Level::Possible => possible_message.into(),
            Level::Safe => unreachable!(),
        };
        Some(Self::new(
            span,
            level.code(definite_code, possible_code),
            message,
            notes,
        ))
    }
}

pub(crate) fn emit_finding<'tcx>(tcx: TyCtxt<'tcx>, finding: &Finding) {
    let sm = tcx.sess.source_map();
    let loc = sm.span_to_diagnostic_string(finding.span);
    let snippet = sm
        .span_to_snippet(finding.span)
        .unwrap_or_else(|_| "unsafe call".to_string())
        .replace('\n', " ");
    println!("warning[{}]: {}", finding.code, finding.message);
    println!("  --> {loc}");
    println!("   |");
    println!("   | {snippet}");
    println!("   | ^ unsafe call here");
    for note in &finding.notes {
        println!("   = {note}");
    }
}
