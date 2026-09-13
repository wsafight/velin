use std::fmt;

#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "lowercase"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
}

impl fmt::Display for Severity {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Error => formatter.write_str("error"),
            Self::Warning => formatter.write_str("warning"),
        }
    }
}

#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    pub severity: Severity,
    pub file: String,
    /// 1-based source line.
    pub line: usize,
    /// 1-based column measured in Unicode scalar values.
    pub column: usize,
    pub message: String,
    pub hint: Option<String>,
}

impl Diagnostic {
    #[must_use]
    pub fn new(
        file: impl Into<String>,
        line: usize,
        column: usize,
        message: impl Into<String>,
    ) -> Self {
        Self {
            severity: Severity::Error,
            file: file.into(),
            line,
            column,
            message: message.into(),
            hint: None,
        }
    }

    #[must_use]
    pub fn warning(
        file: impl Into<String>,
        line: usize,
        column: usize,
        message: impl Into<String>,
    ) -> Self {
        Self {
            severity: Severity::Warning,
            file: file.into(),
            line,
            column,
            message: message.into(),
            hint: None,
        }
    }

    #[must_use]
    pub const fn is_error(&self) -> bool {
        matches!(self.severity, Severity::Error)
    }

    #[must_use]
    pub fn with_hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = Some(hint.into());
        self
    }
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}:{}:{}: {}: {}",
            self.file, self.line, self.column, self.severity, self.message
        )?;
        if let Some(hint) = &self.hint {
            write!(f, "\n  hint: {hint}")?;
        }
        Ok(())
    }
}

impl std::error::Error for Diagnostic {}

#[cfg(test)]
#[path = "diagnostic_tests.rs"]
mod tests;
