//! Physical-line reader: indentation, comments, and blank-line skipping.
//!
//! This is the lexical pre-pass for the statement parser. It turns raw source
//! into a list of [`Line`]s that each carry their 1-based line number, their
//! indentation depth (in levels, not spaces), and the trailing content with any
//! comment stripped. Blank and comment-only lines are dropped so the parser
//! only ever sees meaningful lines.
//!
//! Indentation rules (kept deliberately strict so the grammar is unambiguous):
//!
//! * Indentation is spaces only; a tab in leading whitespace is an error.
//! * One indentation *level* is a fixed [`INDENT_WIDTH`] spaces. A leading
//!   width that is not a multiple of it is an error.
//!
//! Comment stripping respects string literals: a `#` inside a `"..."` string is
//! literal text, not a comment. Escapes (`\"`) are honoured so an escaped quote
//! does not prematurely end the string.

use crate::limits::{MAX_SOURCE_BYTES, MAX_SOURCE_LINES};
use velin_syntax::Diagnostic;

/// The number of spaces that make up one indentation level.
pub const INDENT_WIDTH: usize = 4;

/// A single meaningful source line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Line<'source> {
    /// 1-based source line number, for diagnostics.
    pub number: usize,
    /// Indentation depth in levels (0 = column 1).
    pub indent: usize,
    /// The line content after indentation, with any trailing comment removed
    /// and surrounding whitespace trimmed. Never empty.
    pub content: &'source str,
    /// 1-based column where `content` begins, for anchoring diagnostics into
    /// the embedded expression grammar.
    pub column: usize,
}

/// Splits `source` into meaningful [`Line`]s.
///
/// # Errors
/// Returns a [`Diagnostic`] for a tab in leading whitespace or an indentation
/// width that is not a multiple of [`INDENT_WIDTH`].
pub fn read<'source>(source: &'source str, file: &str) -> Result<Vec<Line<'source>>, Diagnostic> {
    if source.len() > MAX_SOURCE_BYTES {
        return Err(Diagnostic::new(
            file,
            1,
            1,
            format!("source exceeds {} MiB", MAX_SOURCE_BYTES / 1024 / 1024),
        ));
    }
    let mut lines = Vec::new();
    for (offset, raw) in source.lines().enumerate() {
        let number = offset + 1;
        if number > MAX_SOURCE_LINES {
            return Err(Diagnostic::new(
                file,
                number,
                1,
                format!("source exceeds {MAX_SOURCE_LINES} lines"),
            ));
        }
        let spaces = leading_spaces(raw, file, number)?;
        let stripped = strip_comment(&raw[spaces..]);
        let content = stripped.trim_end();
        if content.is_empty() {
            continue; // blank or comment-only line
        }
        if !spaces.is_multiple_of(INDENT_WIDTH) {
            return Err(Diagnostic::new(
                file,
                number,
                spaces + 1,
                format!("indentation must be a multiple of {INDENT_WIDTH} spaces"),
            ));
        }
        lines.push(Line {
            number,
            indent: spaces / INDENT_WIDTH,
            content,
            column: spaces + 1,
        });
    }
    Ok(lines)
}

/// Counts leading spaces, rejecting tabs so indentation depth is unambiguous.
fn leading_spaces(raw: &str, file: &str, number: usize) -> Result<usize, Diagnostic> {
    let mut spaces = 0;
    for byte in raw.bytes() {
        match byte {
            b' ' => spaces += 1,
            b'\t' => {
                return Err(Diagnostic::new(
                    file,
                    number,
                    spaces + 1,
                    "tabs are not allowed in indentation; use spaces",
                ));
            }
            _ => break,
        }
    }
    Ok(spaces)
}

/// Removes a trailing `#` comment, but not one inside a string literal.
fn strip_comment(text: &str) -> &str {
    if !text
        .as_bytes()
        .iter()
        .any(|byte| matches!(byte, b'"' | b'#'))
    {
        return text;
    }

    let mut in_string = false;
    let mut escaped = false;
    let mut hole_depth = 0usize;
    let mut hole_string = false;
    let mut hole_escaped = false;
    let mut chars = text.char_indices().peekable();
    while let Some((index, ch)) = chars.next() {
        if in_string {
            if hole_depth > 0 {
                if hole_string {
                    if hole_escaped {
                        hole_escaped = false;
                    } else if ch == '\\' {
                        hole_escaped = true;
                    } else if ch == '"' {
                        hole_string = false;
                    }
                } else {
                    match ch {
                        '"' => hole_string = true,
                        '[' => hole_depth += 1,
                        ']' => hole_depth -= 1,
                        _ => {}
                    }
                }
            } else if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_string = false;
            } else if ch == '[' {
                if chars.peek().is_some_and(|(_, next)| *next == '[') {
                    chars.next();
                } else {
                    hole_depth = 1;
                }
            }
        } else if ch == '"' {
            in_string = true;
        } else if ch == '#' {
            return &text[..index];
        }
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drops_blank_and_comment_lines_and_measures_indent() {
        let source = "label start:\n\
                      \n\
                      # a comment\n\
                      \x20\x20\x20\x20set hp = 1\n";
        let lines = read(source, "t.velin").unwrap();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].indent, 0);
        assert_eq!(lines[0].content, "label start:");
        assert_eq!(lines[1].indent, 1);
        assert_eq!(lines[1].content, "set hp = 1");
        assert_eq!(lines[1].column, 5);
        assert_eq!(lines[1].number, 4);
    }

    #[test]
    fn hash_inside_string_is_not_a_comment() {
        let lines = read("perform say(\"score #1\") # real comment\n", "t.velin").unwrap();
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].content, "perform say(\"score #1\")");
    }

    #[test]
    fn escaped_quote_does_not_end_string() {
        let lines = read("set s = \"a\\\"b # c\"\n", "t.velin").unwrap();
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].content, "set s = \"a\\\"b # c\"");
    }

    #[test]
    fn hash_inside_an_interpolation_hole_string_is_not_a_comment() {
        let source = r##"perform say("[contains("a#]", "#")]") # real comment"##;
        let lines = read(source, "t.velin").unwrap();
        assert_eq!(
            lines[0].content,
            r##"perform say("[contains("a#]", "#")]")"##
        );
    }

    #[test]
    fn tabs_and_odd_indentation_are_rejected() {
        assert!(read("\tset x = 1\n", "t.velin").is_err());
        assert!(read("  set x = 1\n", "t.velin").is_err()); // 2 spaces, not a multiple of 4
    }

    #[test]
    fn source_size_and_line_count_are_bounded() {
        let oversized = "x".repeat(MAX_SOURCE_BYTES + 1);
        assert!(
            read(&oversized, "t.velin")
                .unwrap_err()
                .message
                .contains("MiB")
        );

        let too_many_lines = "\n".repeat(MAX_SOURCE_LINES + 1);
        assert!(
            read(&too_many_lines, "t.velin")
                .unwrap_err()
                .message
                .contains("lines")
        );
    }

    #[test]
    fn comments_survive_nested_holes_and_escapes() {
        let escaped_hole = r#"perform say("[contains("a\"b#]", "x")]") # keep"#;
        let lines = read(escaped_hole, "t.velin").unwrap();
        assert!(lines[0].content.contains("contains"));
        assert!(!lines[0].content.contains("keep"));

        let nested = r#"perform say("[list([1])] # still string") # real"#;
        let lines = read(nested, "t.velin").unwrap();
        assert!(lines[0].content.contains("list([1])"));
        assert!(!lines[0].content.contains("real"));

        let escaped_bracket = r#"perform say("foo [[ # not") # yes"#;
        let lines = read(escaped_bracket, "t.velin").unwrap();
        assert!(lines[0].content.contains("[["));
        assert!(!lines[0].content.contains("yes"));
    }
}
