//! The statement-syntax error type.
//!
//! Kept in its own module so both the parser and its lexical helpers can build
//! and return it without a cyclic dependency, and so the public error surface
//! is a single small file.

use velin_syntax::Diagnostic;

/// A statement-syntax error with a source location.
///
/// A `ParseError` deliberately carries no file name: the parser works on a
/// source string and the caller knows the path. Attach it when rendering with
/// [`ParseError::into_diagnostic`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    pub line: usize,
    pub column: usize,
    pub message: String,
}

impl ParseError {
    pub(crate) fn new(line: usize, column: usize, message: impl Into<String>) -> Self {
        Self {
            line,
            column,
            message: message.into(),
        }
    }

    /// Converts a lower-level [`Diagnostic`] (e.g. from expression parsing or
    /// the line reader) into a `ParseError`, preserving its location. The
    /// diagnostic's file name is dropped; the caller reattaches a real path in
    /// [`ParseError::into_diagnostic`].
    pub(crate) fn from_diagnostic(diagnostic: Diagnostic) -> Self {
        Self {
            line: diagnostic.line,
            column: diagnostic.column,
            message: diagnostic.message,
        }
    }

    /// Renders this error as a structured [`Diagnostic`] attributed to `file`.
    #[must_use]
    pub fn into_diagnostic(self, file: &str) -> Diagnostic {
        Diagnostic::new(file, self.line, self.column, self.message)
    }
}
