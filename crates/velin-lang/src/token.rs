//! Lexical helpers shared by the statement parser.
//!
//! These are the small, self-contained routines that turn a raw [`Line`]'s
//! text into the pieces the recursive-descent parser assembles: splitting a
//! leading keyword, finding the top-level `=` of an assignment, validating an
//! identifier, and recognising the `:`-terminated headers of compound
//! statements. Keeping them here keeps `parser.rs` focused on grammar.

use crate::error::ParseError;
use crate::lines::Line;
use velin_parse::parse_expression_with_source;
use velin_syntax::{Expr, SharedString};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AssignmentOperator {
    Set,
    Add,
    Subtract,
    Multiply,
    Divide,
}

/// Parses an embedded expression, translating any diagnostic into a
/// `ParseError` at the right location.
pub(crate) fn parse_embedded(
    text: &str,
    source: &SharedString,
    line: usize,
    column: usize,
) -> Result<Expr, ParseError> {
    let trimmed = text.trim_start();
    let leading = text[..text.len() - trimmed.len()].chars().count();
    parse_expression_with_source(trimmed, source, line, column + leading)
        .map_err(ParseError::from_diagnostic)
}

/// Splits a leading identifier-like keyword from the rest of a line.
///
/// The keyword is the maximal leading run of identifier characters
/// (`A-Za-z0-9_`); the rest is everything after it, left-trimmed. This makes
/// `else:` split to `("else", ":")` even though no space separates them, while
/// `set hp = 1` splits to `("set", "hp = 1")`. A line whose first character is
/// not an identifier char (e.g. a bare `= ...`) yields an empty keyword.
pub(crate) fn split_keyword(content: &str) -> (&str, &str) {
    let end = content
        .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .unwrap_or(content.len());
    (&content[..end], content[end..].trim_start())
}

/// Splits a top-level assignment, including `+=`, `-=`, `*=`, and `/=`.
pub(crate) fn split_assignment<'source>(
    line: &Line<'source>,
) -> Result<(&'source str, &'source str, usize, AssignmentOperator), ParseError> {
    let content = &line.content;
    let bytes = content.as_bytes();
    let mut index = 0;
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    while index < bytes.len() {
        if in_string {
            if escaped {
                escaped = false;
            } else if bytes[index] == b'\\' {
                escaped = true;
            } else if bytes[index] == b'"' {
                in_string = false;
            }
            index += 1;
            continue;
        }
        match bytes[index] {
            b'"' => in_string = true,
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => depth = depth.saturating_sub(1),
            _ => {}
        }
        if bytes[index] == b'=' {
            let prev = index.checked_sub(1).map(|i| bytes[i]);
            let next = bytes.get(index + 1).copied();
            // Skip comparison/relational operators: ==, !=, <=, >=.
            let part_of_comparison =
                next == Some(b'=') || matches!(prev, Some(b'!' | b'<' | b'>' | b'='));
            if depth == 0 && !part_of_comparison {
                let (target_end, operator) = match prev {
                    Some(b'+') => (index - 1, AssignmentOperator::Add),
                    Some(b'-') => (index - 1, AssignmentOperator::Subtract),
                    Some(b'*') => (index - 1, AssignmentOperator::Multiply),
                    Some(b'/') => (index - 1, AssignmentOperator::Divide),
                    _ => (index, AssignmentOperator::Set),
                };
                let target = content[..target_end].trim();
                let value = &content[index + 1..];
                let value_column = line.column + content[..=index].chars().count();
                return Ok((target, value, value_column, operator));
            }
        }
        index += 1;
    }
    Err(ParseError::new(
        line.number,
        line.column,
        "expected `name = value`",
    ))
}

/// Validates an identifier: non-empty, ASCII-alphanumeric/underscore, not
/// starting with a digit.
pub(crate) fn expect_identifier(line: &Line<'_>, text: &str) -> Result<String, ParseError> {
    let valid = !text.is_empty()
        && text
            .chars()
            .enumerate()
            .all(|(i, c)| c == '_' || c.is_ascii_alphabetic() || (i > 0 && c.is_ascii_digit()));
    if valid {
        Ok(text.to_owned())
    } else {
        Err(ParseError::new(
            line.number,
            line.column,
            format!("expected an identifier, found `{text}`"),
        ))
    }
}

/// Parses a `name:` header (used by `label`), rejecting a missing colon.
pub(crate) fn expect_header_name(line: &Line<'_>, rest: &str) -> Result<String, ParseError> {
    let text = rest.trim();
    let name = text.strip_suffix(':').ok_or_else(|| {
        ParseError::new(line.number, line.column, "expected `:` after label name")
    })?;
    expect_identifier(line, name.trim())
}

/// Parses a `<expr>:` header, returning the expression text and its column.
pub(crate) fn expect_colon_header<'a>(
    line: &Line<'_>,
    rest: &'a str,
) -> Result<(&'a str, usize), ParseError> {
    let trimmed = rest.trim_end();
    let expr = trimmed
        .strip_suffix(':')
        .ok_or_else(|| ParseError::new(line.number, line.column, "expected `:` to open a block"))?;
    // Column of the expression: past the keyword to the first non-space of rest.
    let rest_offset = line.content.len() - rest.len();
    let leading = rest.len() - rest.trim_start().len();
    let column = line.column + line.content[..rest_offset + leading].chars().count();
    Ok((expr, column))
}

/// Validates a bare `keyword:` header such as `else:`.
pub(crate) fn expect_bare_header(
    line: &Line<'_>,
    rest: &str,
    keyword: &str,
) -> Result<(), ParseError> {
    if rest.trim() == ":" {
        Ok(())
    } else {
        Err(ParseError::new(
            line.number,
            line.column,
            format!("`{keyword}` takes no condition and must end with `:`"),
        ))
    }
}

#[cfg(test)]
#[path = "token_tests.rs"]
mod tests;
