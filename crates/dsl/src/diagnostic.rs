use crate::Span;
use std::fmt;

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct Diagnostic {
    /// Byte offsets in the DSL input.
    pub span: Span,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<DiagnosticContext>,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct DiagnosticContext {
    /// One-based line and character column in the DSL input, not the Rust file.
    pub line: usize,
    pub column: usize,
    pub source_line: String,
    pub highlight_len: usize,
}

impl Diagnostic {
    pub fn new(span: Span, message: impl Into<String>) -> Self {
        Self {
            span,
            message: message.into(),
            hint: None,
            context: None,
        }
    }

    pub fn with_hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = Some(hint.into());
        self
    }

    pub fn with_source(mut self, source: &str) -> Self {
        let boundary = |offset: usize| {
            let mut offset = offset.min(source.len());
            while !source.is_char_boundary(offset) {
                offset -= 1;
            }
            offset
        };
        let start = boundary(self.span.start);
        let line_start = source[..start].rfind('\n').map_or(0, |i| i + 1);
        let line_end = source[start..]
            .find('\n')
            .map_or(source.len(), |i| start + i);
        let end = boundary(self.span.end).max(start).min(line_end);
        self.context = Some(DiagnosticContext {
            line: source[..start].bytes().filter(|b| *b == b'\n').count() + 1,
            column: source[line_start..start].chars().count() + 1,
            source_line: source[line_start..line_end].into(),
            highlight_len: source[start..end].chars().count().max(1),
        });
        self
    }
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(context) = &self.context {
            writeln!(
                f,
                "DSL 第 {} 行，第 {} 列：{}",
                context.line, context.column, self.message
            )?;
            writeln!(f, "  {}", context.source_line)?;
            write!(
                f,
                "  {}{}",
                " ".repeat(context.column - 1),
                "^".repeat(context.highlight_len)
            )?;
        } else {
            write!(f, "DSL {:?}: {}", self.span, self.message)?;
        }
        if let Some(hint) = &self.hint {
            write!(f, "\n提示：{hint}")?;
        }
        Ok(())
    }
}
