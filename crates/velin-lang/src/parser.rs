//! Indentation-driven recursive-descent parser for the statement language.
//!
//! Input is the meaningful [`Line`]s from [`crate::lines`]; each carries an
//! indentation depth. A *block* is a run of consecutive lines at the same
//! indent; a compound statement's body is the block indented one level deeper.
//! The parser is a straightforward cursor over the line list.
//!
//! Every expression (assignment right-hand side, condition, host argument) is
//! delegated to [`velin_parse::parse_expression`], so operator precedence and
//! value literals are exactly the core language's — this file only handles the
//! statement scaffolding around them.

use crate::ast::{Branch, Condition, Stmt};
use crate::error::ParseError;
use crate::limits::MAX_STATEMENT_DEPTH;
use crate::lines::{self, Line};
use crate::token::{
    expect_bare_header, expect_colon_header, expect_header_name, expect_identifier, parse_embedded,
    split_eq, split_keyword,
};
use velin_parse::parse_expression_list_with_source;
use velin_syntax::{Expr, SharedString};

mod recovery;

/// A best-effort statement parse for editor tooling.
///
/// Valid statements around syntax errors are retained, while every recovered
/// error is reported with its original source location. This result must not
/// be lowered or executed when `errors` is non-empty.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecoveredProgram {
    pub statements: Vec<Stmt>,
    pub errors: Vec<ParseError>,
}

/// Parses `source` into a flat statement list (compound statements nest their
/// bodies).
///
/// # Errors
/// Returns a [`ParseError`] for bad indentation, an unknown or malformed
/// statement, or an invalid embedded expression.
pub fn parse(source: &str) -> Result<Vec<Stmt>, ParseError> {
    let lines = lines::read(source, crate::FILE).map_err(ParseError::from_diagnostic)?;
    let mut parser = Parser {
        lines: &lines,
        at: 0,
        source: SharedString::from(crate::FILE),
    };
    let statements = parser.block(0)?;
    if parser.at != lines.len() {
        let line = &lines[parser.at];
        return Err(ParseError::new(
            line.number,
            line.column,
            "unexpected indentation",
        ));
    }
    Ok(statements)
}

/// Parses as much of `source` as possible for editor features.
///
/// Unlike [`parse`], this keeps valid sibling statements after a malformed
/// statement and can return multiple syntax errors. The strict parser remains
/// the only parser used by compilation and execution.
pub fn parse_recovering(source: &str) -> RecoveredProgram {
    let lines = match lines::read(source, crate::FILE) {
        Ok(lines) => lines,
        Err(diagnostic) => {
            return RecoveredProgram {
                statements: Vec::new(),
                errors: vec![ParseError::from_diagnostic(diagnostic)],
            };
        }
    };
    let mut parser = Parser {
        lines: &lines,
        at: 0,
        source: SharedString::from(crate::FILE),
    };
    let mut errors = Vec::new();
    let statements = parser.recovering_block(0, &mut errors);
    RecoveredProgram { statements, errors }
}

struct Parser<'lines, 'source> {
    lines: &'lines [Line<'source>],
    at: usize,
    source: SharedString,
}

impl<'lines, 'source> Parser<'lines, 'source> {
    /// Parses every consecutive statement at exactly `indent`, stopping at the
    /// first line that is shallower (block ends) or deeper (unexpected).
    fn block(&mut self, indent: usize) -> Result<Vec<Stmt>, ParseError> {
        if indent > MAX_STATEMENT_DEPTH {
            let line = self.peek().expect("nested block starts with a line");
            return Err(ParseError::new(
                line.number,
                line.column,
                format!("statement nesting exceeds {MAX_STATEMENT_DEPTH}"),
            ));
        }
        let mut statements = Vec::new();
        while let Some(line) = self.peek() {
            if line.indent < indent {
                break;
            }
            if line.indent > indent {
                return Err(ParseError::new(
                    line.number,
                    line.column,
                    "unexpected indentation",
                ));
            }
            statements.push(self.statement(indent)?);
        }
        Ok(statements)
    }

    /// Parses one statement (and, for compound statements, its nested body).
    fn statement(&mut self, indent: usize) -> Result<Stmt, ParseError> {
        let line = self
            .advance()
            .expect("statement called with a line present");
        let (keyword, rest) = split_keyword(line.content);
        match keyword {
            "label" => self.parse_label(line, rest, indent),
            "default" => Self::parse_binding(line, rest, true, &self.source),
            "set" => Self::parse_binding(line, rest, false, &self.source),
            "perform" => Self::parse_perform(line, rest, None, &self.source),
            "if" => self.parse_if(line, rest, indent),
            "while" => self.parse_while(line, rest, indent),
            "jump" => Self::parse_jump(line, rest),
            _ => Self::parse_bare_assign_or_bind(line, &self.source),
        }
    }

    fn parse_label(
        &mut self,
        line: &Line<'_>,
        rest: &str,
        indent: usize,
    ) -> Result<Stmt, ParseError> {
        let name = expect_header_name(line, rest)?;
        // A label may optionally own an indented body; the following block one
        // level deeper is the statements it introduces. When absent, the label
        // is just a marker before the statements that follow at the same level.
        let body = match self.peek() {
            Some(next) if next.indent == indent + 1 => self.block(indent + 1)?,
            _ => Vec::new(),
        };
        Ok(Stmt::Label {
            name,
            body,
            line: line.number,
        })
    }

    /// `default name = expr` / `set name = expr`.
    fn parse_binding(
        line: &Line<'_>,
        rest: &str,
        is_default: bool,
        source: &SharedString,
    ) -> Result<Stmt, ParseError> {
        let (name, value) = Self::split_assignment(line, rest, source)?;
        if is_default {
            Ok(Stmt::Default {
                name,
                value,
                line: line.number,
            })
        } else {
            Ok(Stmt::Set {
                name,
                value,
                line: line.number,
            })
        }
    }

    /// `perform cmd(args...)`, with an optional `bind` variable supplied by the
    /// caller for the `name = perform ...` form.
    fn parse_perform(
        line: &Line<'_>,
        rest: &str,
        bind: Option<String>,
        source: &SharedString,
    ) -> Result<Stmt, ParseError> {
        let (command, arguments) = Self::parse_call(line, rest, source)?;
        Ok(Stmt::Perform {
            command,
            arguments,
            bind,
            line: line.number,
        })
    }

    /// A line with no leading keyword: either `name = perform ...` (a bound
    /// host effect) or `name = expr` (an assignment without the `set` keyword,
    /// accepted as sugar).
    fn parse_bare_assign_or_bind(
        line: &Line<'_>,
        source: &SharedString,
    ) -> Result<Stmt, ParseError> {
        let (name, rhs, rhs_column) = split_eq(line)?;
        let leading = rhs[..rhs.len() - rhs.trim_start().len()].chars().count();
        let rhs = rhs.trim_start();
        let rhs_column = rhs_column + leading;
        let (keyword, rest) = split_keyword(rhs);
        if keyword == "perform" {
            // Re-anchor the command portion at its real column.
            let command_line = Line {
                number: line.number,
                indent: line.indent,
                content: rhs,
                column: rhs_column,
            };
            return Self::parse_perform(&command_line, rest, Some(name), source);
        }
        let value = parse_embedded(rhs, source, line.number, rhs_column)?;
        Ok(Stmt::Set {
            name,
            value,
            line: line.number,
        })
    }

    fn parse_if(&mut self, line: &Line<'_>, rest: &str, indent: usize) -> Result<Stmt, ParseError> {
        let mut branches = vec![self.parse_branch(line, rest, indent)?];
        let mut otherwise = None;
        while let Some(next) = self.peek() {
            if next.indent != indent {
                break;
            }
            let (keyword, rest) = split_keyword(next.content);
            match keyword {
                "elif" => {
                    let header = self.advance().expect("peeked line present");
                    branches.push(self.parse_branch(header, rest, indent)?);
                }
                "else" => {
                    let header = self.advance().expect("peeked line present");
                    expect_bare_header(header, rest, "else")?;
                    otherwise = Some(self.body(indent, header)?);
                    break;
                }
                _ => break,
            }
        }
        Ok(Stmt::If {
            branches,
            otherwise,
        })
    }

    /// Parses a guarded header (`if`/`elif` cond `:`) and its indented body.
    fn parse_branch(
        &mut self,
        header: &Line<'_>,
        rest: &str,
        indent: usize,
    ) -> Result<Branch, ParseError> {
        let condition = Self::parse_condition(header, rest, &self.source)?;
        let body = self.body(indent, header)?;
        Ok(Branch { condition, body })
    }

    fn parse_while(
        &mut self,
        line: &Line<'_>,
        rest: &str,
        indent: usize,
    ) -> Result<Stmt, ParseError> {
        let condition = Self::parse_condition(line, rest, &self.source)?;
        let body = self.body(indent, line)?;
        Ok(Stmt::While { condition, body })
    }

    fn parse_jump(line: &Line<'_>, rest: &str) -> Result<Stmt, ParseError> {
        let label = expect_identifier(line, rest.trim())?;
        Ok(Stmt::Jump {
            label,
            line: line.number,
        })
    }

    /// Parses the `<expr>:` guard shared by `if`/`elif`/`while`.
    fn parse_condition(
        line: &Line<'_>,
        rest: &str,
        source: &SharedString,
    ) -> Result<Condition, ParseError> {
        let (expr_text, expr_column) = expect_colon_header(line, rest)?;
        let expr = parse_embedded(expr_text, source, line.number, expr_column)?;
        Ok(Condition {
            expr,
            line: line.number,
        })
    }

    /// Parses the indented body block that must follow a compound-statement
    /// header, erroring if none is present.
    fn body(&mut self, indent: usize, header: &Line<'_>) -> Result<Vec<Stmt>, ParseError> {
        match self.peek() {
            Some(next) if next.indent == indent + 1 => self.block(indent + 1),
            _ => Err(ParseError::new(
                header.number,
                header.column,
                "expected an indented block after this line",
            )),
        }
    }

    /// Splits `name = expr` at the top-level `=`, parsing the right side.
    fn split_assignment(
        line: &Line<'_>,
        rest: &str,
        source: &SharedString,
    ) -> Result<(String, Expr), ParseError> {
        let rest_offset = line.content.len() - rest.len();
        let synthetic = Line {
            number: line.number,
            indent: line.indent,
            content: rest,
            // `rest` starts one keyword+space past the content column.
            column: line.column + line.content[..rest_offset].chars().count(),
        };
        let (name, value_text, value_column) = split_eq(&synthetic)?;
        let value = parse_embedded(value_text, source, line.number, value_column)?;
        Ok((name, value))
    }

    /// Parses `cmd(arg, arg, ...)` into a command name and argument expressions.
    fn parse_call(
        line: &Line<'_>,
        rest: &str,
        source: &SharedString,
    ) -> Result<(String, Vec<Expr>), ParseError> {
        let text = rest.trim();
        let open = text.find('(').ok_or_else(|| {
            ParseError::new(line.number, line.column, "expected `(` after host command")
        })?;
        if !text.ends_with(')') {
            return Err(ParseError::new(
                line.number,
                line.column,
                "expected `)` to close host command arguments",
            ));
        }
        let command = expect_identifier(line, text[..open].trim())?;
        let inner = &text[open + 1..text.len() - 1];
        let arguments = if inner.trim().is_empty() {
            Vec::new()
        } else {
            let rest_offset = line.content.len() - rest.len();
            let text_offset = rest.len() - rest.trim_start().len();
            let inner_offset = rest_offset + text_offset + open + 1;
            let inner_column = line.column + line.content[..inner_offset].chars().count();
            parse_expression_list_with_source(inner, source, line.number, inner_column)
                .map_err(ParseError::from_diagnostic)?
        };
        Ok((command, arguments))
    }

    fn peek(&self) -> Option<&'lines Line<'source>> {
        self.lines.get(self.at)
    }

    fn advance(&mut self) -> Option<&'lines Line<'source>> {
        let line = self.lines.get(self.at);
        if line.is_some() {
            self.at += 1;
        }
        line
    }
}

#[cfg(test)]
#[path = "parser_tests.rs"]
mod tests;
