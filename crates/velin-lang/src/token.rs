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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AssignmentTarget<'source> {
    Identifier(&'source str),
    General(&'source str),
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

/// Removes one exact leading keyword without scanning the following token.
pub(crate) fn strip_keyword<'source>(content: &'source str, keyword: &str) -> Option<&'source str> {
    let rest = content.strip_prefix(keyword)?;
    if rest
        .chars()
        .next()
        .is_some_and(|character| character.is_ascii_alphanumeric() || character == '_')
    {
        return None;
    }
    Some(rest.trim_start())
}

/// Splits a top-level assignment, including `+=`, `-=`, `*=`, and `/=`.
pub(crate) fn split_assignment<'source>(
    line: &Line<'source>,
) -> Result<
    (
        AssignmentTarget<'source>,
        &'source str,
        usize,
        AssignmentOperator,
    ),
    ParseError,
> {
    let content = &line.content;
    let bytes = content.as_bytes();
    if let Some(index) = content.find('=') {
        let prev = index.checked_sub(1).map(|position| bytes[position]);
        let next = bytes.get(index + 1).copied();
        let part_of_comparison =
            next == Some(b'=') || matches!(prev, Some(b'!' | b'<' | b'>' | b'='));
        if !part_of_comparison {
            let (target_end, operator) = assignment_operator(bytes, index);
            if let Some(target) = simple_assignment_target(content, target_end) {
                let (value, value_column) =
                    trim_assignment_value(&content[index + 1..], line.column + index + 1);
                return Ok((target, value, value_column, operator));
            }
        }
    }
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
                return Ok(assignment_parts(line, index));
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

fn assignment_parts<'source>(
    line: &Line<'source>,
    index: usize,
) -> (
    AssignmentTarget<'source>,
    &'source str,
    usize,
    AssignmentOperator,
) {
    let content = line.content;
    let (target_end, operator) = assignment_operator(content.as_bytes(), index);
    let target = content[..target_end].trim();
    let target = if is_identifier(target) {
        AssignmentTarget::Identifier(target)
    } else {
        AssignmentTarget::General(target)
    };
    let column_after_operator = line.column + content[..=index].chars().count();
    let (value, value_column) = trim_assignment_value(&content[index + 1..], column_after_operator);
    (target, value, value_column, operator)
}

fn trim_assignment_value(value: &str, column: usize) -> (&str, usize) {
    let trimmed = value.trim_start();
    let leading = &value[..value.len() - trimmed.len()];
    let leading_columns = if leading.is_ascii() {
        leading.len()
    } else {
        leading.chars().count()
    };
    (trimmed, column + leading_columns)
}

fn assignment_operator(bytes: &[u8], index: usize) -> (usize, AssignmentOperator) {
    match index.checked_sub(1).map(|position| bytes[position]) {
        Some(b'+') => (index - 1, AssignmentOperator::Add),
        Some(b'-') => (index - 1, AssignmentOperator::Subtract),
        Some(b'*') => (index - 1, AssignmentOperator::Multiply),
        Some(b'/') => (index - 1, AssignmentOperator::Divide),
        _ => (index, AssignmentOperator::Set),
    }
}

fn simple_assignment_target(content: &str, target_end: usize) -> Option<AssignmentTarget<'_>> {
    let target = content[..target_end].trim_ascii();
    let (&first, rest) = target.as_bytes().split_first()?;
    if (first == b'_' || first.is_ascii_alphabetic())
        && rest
            .iter()
            .all(|byte| *byte == b'_' || byte.is_ascii_alphanumeric())
    {
        Some(AssignmentTarget::Identifier(target))
    } else {
        None
    }
}

/// Validates an identifier: non-empty, ASCII-alphanumeric/underscore, not
/// starting with a digit.
pub(crate) fn expect_identifier(line: &Line<'_>, text: &str) -> Result<String, ParseError> {
    if is_identifier(text) {
        Ok(text.to_owned())
    } else {
        Err(ParseError::new(
            line.number,
            line.column,
            format!("expected an identifier, found `{text}`"),
        ))
    }
}

fn is_identifier(text: &str) -> bool {
    !text.is_empty()
        && text
            .chars()
            .enumerate()
            .all(|(i, c)| c == '_' || c.is_ascii_alphabetic() || (i > 0 && c.is_ascii_digit()))
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
