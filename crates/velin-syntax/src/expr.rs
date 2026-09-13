#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};
use std::fmt::Write as _;
use std::ops::Deref;
use std::sync::Arc;

pub use crate::data::Builtin;

/// A source location: file, 1-based line, 1-based column.
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Span {
    pub source: SharedString,
    pub line: usize,
    pub column: usize,
}

impl Span {
    #[must_use]
    pub fn new(line: usize, column: usize) -> Self {
        Self::in_source("", line, column)
    }

    #[must_use]
    pub fn in_source(source: impl Into<SharedString>, line: usize, column: usize) -> Self {
        Self {
            source: source.into(),
            line,
            column,
        }
    }
}

/// A pure, host-agnostic expression tree.
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Expr {
    /// An expression node paired with its exact source position. Parsers wrap
    /// every produced node; hand-built expressions may remain unspanned.
    Spanned {
        span: Span,
        expression: Box<Expr>,
    },
    Invoke {
        function: Builtin,
        arguments: Vec<Expr>,
    },
    Value(Value),
    Variable(String),
    Unary {
        op: UnaryOp,
        value: Box<Expr>,
    },
    Binary {
        left: Box<Expr>,
        op: BinaryOp,
        right: Box<Expr>,
    },
    /// A string built from literal text and interpolated expression holes.
    ///
    /// Produced only when a string literal contains at least one `[expr]` hole;
    /// a plain literal string stays [`Expr::Value`] so nothing regresses. Each
    /// hole is evaluated and rendered deterministically (no locale, no float)
    /// then concatenated with the surrounding literal parts.
    Interpolate {
        parts: Vec<StrPart>,
    },
}

impl Expr {
    /// Attaches a source position to an expression node.
    #[must_use]
    pub fn spanned(self, span: Span) -> Self {
        Self::Spanned {
            span,
            expression: Box::new(self),
        }
    }

    /// Returns the outer source position, if this node came from a parser.
    #[must_use]
    pub const fn span(&self) -> Option<&Span> {
        match self {
            Self::Spanned { span, .. } => Some(span),
            _ => None,
        }
    }

    /// Removes any outer position wrapper for structural inspection.
    #[must_use]
    pub fn unspanned(&self) -> &Self {
        match self {
            Self::Spanned { expression, .. } => expression.unspanned(),
            expression => expression,
        }
    }

    /// Consumes an expression and removes all outer position wrappers.
    #[must_use]
    pub fn into_unspanned(self) -> Self {
        match self {
            Self::Spanned { expression, .. } => expression.into_unspanned(),
            expression => expression,
        }
    }
}

/// One piece of an interpolated string: either fixed text or an expression hole.
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StrPart {
    Literal(String),
    Hole(Box<Expr>),
}

/// Cheaply cloned string storage used by [`Value::String`] and [`Span`].
///
/// The wrapper keeps the serialized representation as a string while allowing the VM
/// to mutate a uniquely owned string in place for `text = text + suffix`.
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(transparent))]
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct SharedString(Arc<String>);

impl SharedString {
    #[must_use]
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }

    /// Returns mutable access, cloning the backing string only when another
    /// value or machine snapshot still shares it.
    pub fn make_mut(&mut self) -> &mut String {
        Arc::make_mut(&mut self.0)
    }

    /// Unwraps unique storage or copies the text when it is still shared.
    #[must_use]
    pub fn into_string(self) -> String {
        Arc::try_unwrap(self.0).unwrap_or_else(|shared| (*shared).clone())
    }
}

impl Deref for SharedString {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        self.as_str()
    }
}

impl AsRef<str> for SharedString {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl PartialEq<&str> for SharedString {
    fn eq(&self, other: &&str) -> bool {
        self.as_str() == *other
    }
}

impl PartialEq<str> for SharedString {
    fn eq(&self, other: &str) -> bool {
        self.as_str() == other
    }
}

impl From<String> for SharedString {
    fn from(value: String) -> Self {
        Self(Arc::new(value))
    }
}

impl From<&str> for SharedString {
    fn from(value: &str) -> Self {
        Self(Arc::new(value.to_owned()))
    }
}

/// The closed set of deterministic values the language can hold.
///
/// There is deliberately no floating-point variant: the language is meant to
/// be fully deterministic and reproducible across platforms.
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(untagged))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Value {
    Integer(i64),
    Boolean(bool),
    String(SharedString),
    List(std::sync::Arc<Vec<Value>>),
    Record(std::sync::Arc<std::collections::BTreeMap<String, Value>>),
}

impl Value {
    #[must_use]
    pub const fn type_name(&self) -> &'static str {
        match self {
            Self::Integer(_) => "integer",
            Self::Boolean(_) => "boolean",
            Self::String(_) => "string",
            Self::List(_) => "list",
            Self::Record(_) => "record",
        }
    }

    /// Renders the value as a deterministic, locale-free string for text
    /// interpolation.
    ///
    /// The mapping is fixed across platforms: integers in base 10, booleans as
    /// `true`/`false`, strings verbatim, and compounds in a stable bracketed
    /// form (`[a, b]`, `{key: value}`). There is deliberately no float variant,
    /// so a rendered value never depends on locale or rounding.
    #[must_use]
    pub fn to_display(&self) -> String {
        let mut output = String::new();
        append_display(self, &mut output, None).unwrap_or(());
        output
    }

    /// Renders a validated value without allowing the resulting text to exceed
    /// the shared data budget.
    ///
    /// # Errors
    /// Returns an error if the input value is outside the data budget or its
    /// rendered representation would exceed 1 MiB.
    pub fn try_to_display(&self) -> Result<String, &'static str> {
        let mut output = String::new();
        self.append_to_display(&mut output, crate::data::MAX_DATA_TEXT_BYTES)?;
        Ok(output)
    }

    /// Returns the byte length of the deterministic display representation.
    ///
    /// Validation is performed once before walking the tree, allowing callers
    /// to reserve the exact output size without temporary scalar strings.
    pub fn display_len(&self) -> Result<usize, &'static str> {
        self.validate_data()?;
        self.display_len_known()
    }

    /// Returns the display length for a value whose metrics were already
    /// validated by the caller. This skips a second validation walk.
    pub fn display_len_known(&self) -> Result<usize, &'static str> {
        display_len(self)
    }

    /// Appends the deterministic display representation without allowing the
    /// complete destination buffer to exceed `limit` bytes.
    ///
    /// # Errors
    /// Returns an error if this value is invalid or the resulting buffer would
    /// exceed `limit`.
    pub fn append_to_display(&self, output: &mut String, limit: usize) -> Result<(), &'static str> {
        self.validate_data()?;
        self.append_to_display_known(output, limit)
    }

    /// Appends a value whose data metrics were already checked by the caller.
    /// The destination limit is still enforced.
    pub fn append_to_display_known(
        &self,
        output: &mut String,
        limit: usize,
    ) -> Result<(), &'static str> {
        let length = self.display_len_known()?;
        let total = output
            .len()
            .checked_add(length)
            .ok_or("rendered text exceeds 1 MiB")?;
        if total > limit {
            return Err("rendered text exceeds 1 MiB");
        }
        output
            .try_reserve(length)
            .map_err(|_| "rendered text allocation failed")?;
        append_display(self, output, Some(limit))
    }
}

fn display_len(value: &Value) -> Result<usize, &'static str> {
    match value {
        Value::Integer(number) => Ok(integer_display_len(*number)),
        Value::Boolean(boolean) => Ok(if *boolean { 4 } else { 5 }),
        Value::String(text) => Ok(text.len()),
        Value::List(items) => {
            let mut length = 2usize;
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    length = length.checked_add(2).ok_or("rendered text size overflow")?;
                }
                length = length
                    .checked_add(display_len(item)?)
                    .ok_or("rendered text size overflow")?;
            }
            Ok(length)
        }
        Value::Record(fields) => {
            let mut length = 2usize;
            for (index, (key, field)) in fields.iter().enumerate() {
                if index > 0 {
                    length = length.checked_add(2).ok_or("rendered text size overflow")?;
                }
                length = length
                    .checked_add(key.len())
                    .and_then(|length| length.checked_add(2))
                    .and_then(|length| length.checked_add(display_len(field).ok()?))
                    .ok_or("rendered text size overflow")?;
            }
            Ok(length)
        }
    }
}

fn integer_display_len(number: i64) -> usize {
    let mut magnitude = number.unsigned_abs();
    let mut length = usize::from(number < 0);
    if magnitude == 0 {
        return 1;
    }
    while magnitude != 0 {
        length += 1;
        magnitude /= 10;
    }
    length
}

fn append_display(
    value: &Value,
    output: &mut String,
    limit: Option<usize>,
) -> Result<(), &'static str> {
    match value {
        Value::Integer(number) => push_integer(output, *number, limit)?,
        Value::Boolean(boolean) => {
            push_display(output, if *boolean { "true" } else { "false" }, limit)?;
        }
        Value::String(text) => push_display(output, text, limit)?,
        Value::List(items) => {
            push_display(output, "[", limit)?;
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    push_display(output, ", ", limit)?;
                }
                append_display(item, output, limit)?;
            }
            push_display(output, "]", limit)?;
        }
        Value::Record(fields) => {
            push_display(output, "{", limit)?;
            for (index, (key, field)) in fields.iter().enumerate() {
                if index > 0 {
                    push_display(output, ", ", limit)?;
                }
                push_display(output, key, limit)?;
                push_display(output, ": ", limit)?;
                append_display(field, output, limit)?;
            }
            push_display(output, "}", limit)?;
        }
    }
    Ok(())
}

fn push_integer(
    output: &mut String,
    number: i64,
    limit: Option<usize>,
) -> Result<(), &'static str> {
    let length = integer_display_len(number);
    if limit.is_some_and(|limit| {
        output
            .len()
            .checked_add(length)
            .is_none_or(|total| total > limit)
    }) {
        return Err("rendered text exceeds 1 MiB");
    }
    write!(output, "{number}").map_err(|_| "rendered text allocation failed")
}

fn push_display(output: &mut String, text: &str, limit: Option<usize>) -> Result<(), &'static str> {
    if limit.is_some_and(|limit| {
        output
            .len()
            .checked_add(text.len())
            .is_none_or(|length| length > limit)
    }) {
        return Err("rendered text exceeds 1 MiB");
    }
    output.push_str(text);
    Ok(())
}

#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnaryOp {
    Negate,
    Not,
}

#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinaryOp {
    Add,
    Subtract,
    Multiply,
    Divide,
    Equal,
    NotEqual,
    Less,
    LessEqual,
    Greater,
    GreaterEqual,
    And,
    Or,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spans_record_source_line_and_column() {
        let span = Span::new(3, 8);
        assert_eq!(span.line, 3);
        assert_eq!(span.column, 8);
        assert_eq!(span.source, "");
        let located = Span::in_source("script.velin", 1, 2);
        assert_eq!(located.source, "script.velin");
        assert_eq!(located.line, 1);
        assert_eq!(located.column, 2);
    }

    #[test]
    fn value_type_names_are_stable() {
        assert_eq!(Value::Integer(1).type_name(), "integer");
        assert_eq!(Value::Boolean(true).type_name(), "boolean");
        assert_eq!(Value::String(String::new().into()).type_name(), "string");
        assert_eq!(
            Value::List(std::sync::Arc::new(Vec::new())).type_name(),
            "list"
        );
        assert_eq!(
            Value::Record(std::sync::Arc::new(std::collections::BTreeMap::new())).type_name(),
            "record"
        );
    }

    #[test]
    fn to_display_is_deterministic_and_locale_free() {
        assert_eq!(Value::Integer(-42).to_display(), "-42");
        assert_eq!(Value::Boolean(false).to_display(), "false");
        assert_eq!(Value::String("hi".into()).to_display(), "hi");
        let list = Value::List(std::sync::Arc::new(vec![
            Value::Integer(1),
            Value::String("x".into()),
        ]));
        assert_eq!(list.to_display(), "[1, x]");
        let record = Value::Record(std::sync::Arc::new(std::collections::BTreeMap::from([
            ("b".to_owned(), Value::Integer(2)),
            ("a".to_owned(), Value::Boolean(true)),
        ])));
        // BTreeMap keeps keys sorted, so the rendering is stable.
        assert_eq!(record.to_display(), "{a: true, b: 2}");
    }

    #[test]
    fn bounded_display_counts_collection_punctuation() {
        let value = Value::List(std::sync::Arc::new(vec![Value::String(
            "x".repeat(crate::data::MAX_DATA_TEXT_BYTES).into(),
        )]));
        assert!(value.validate_data().is_ok());
        assert_eq!(value.try_to_display(), Err("rendered text exceeds 1 MiB"));
    }

    #[test]
    fn display_length_matches_rendered_bytes_without_scalar_temporaries() {
        let value = Value::Record(std::sync::Arc::new(std::collections::BTreeMap::from([
            ("answer".to_owned(), Value::Integer(i64::MIN)),
            ("ok".to_owned(), Value::Boolean(true)),
        ])));
        let rendered = value.to_display();
        assert_eq!(value.display_len().unwrap(), rendered.len());
        let mut output = String::new();
        value
            .append_to_display_known(&mut output, crate::data::MAX_DATA_TEXT_BYTES)
            .unwrap();
        assert_eq!(output, rendered);
    }
}
