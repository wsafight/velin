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
    AssignmentOperator, expect_bare_header, expect_colon_header, expect_header_name,
    expect_identifier, parse_embedded, split_assignment, split_keyword, strip_keyword,
};
use velin_parse::parse_expression_list_with_source;
use velin_syntax::{Expr, SharedString};

mod assignments;
mod declarations;
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
        if let Some(rest) = line.content.strip_prefix("set ") {
            return Self::parse_binding(line, rest.trim_start(), false, &self.source);
        }
        if let Some(rest) = line.content.strip_prefix("default ") {
            return Self::parse_binding(line, rest.trim_start(), true, &self.source);
        }
        let (keyword, rest) = split_keyword(line.content);
        match keyword {
            "import" => Self::parse_import(line, rest),
            "fn" => self.parse_function(line, rest, indent, false),
            "export" => self.parse_export(line, rest, indent),
            "return" => Self::parse_return(line, rest, &self.source),
            "label" => self.parse_label(line, rest, indent),
            "default" => Self::parse_binding(line, rest, true, &self.source),
            "set" => Self::parse_binding(line, rest, false, &self.source),
            "perform" => Self::parse_perform(line, rest, None, &self.source),
            "if" => self.parse_if(line, rest, indent),
            "while" => self.parse_while(line, rest, indent),
            "for" => self.parse_for(line, rest, indent),
            "break" => Self::parse_loop_control(line, rest, true),
            "continue" => Self::parse_loop_control(line, rest, false),
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
        let rest_offset = line.content.len() - rest.len();
        let synthetic = Line {
            number: line.number,
            indent: line.indent,
            content: rest,
            column: line.column + line.content[..rest_offset].chars().count(),
        };
        let (target, value_text, value_column, operator) = split_assignment(&synthetic)?;
        let (name, index) = Self::parse_assignment_target(&synthetic, target, source)?;
        if !is_default && let Some(call_rest) = strip_keyword(value_text, "call") {
            if operator != AssignmentOperator::Set || index.is_some() {
                return Err(ParseError::new(
                    line.number,
                    line.column,
                    "a function result requires a simple variable assignment",
                ));
            }
            let (module, function, arguments) =
                Self::parse_function_call(line, call_rest, source, value_column)?;
            return Ok(Stmt::Call {
                module,
                function,
                arguments,
                bind: name,
                line: line.number,
            });
        }
        let mut value = parse_embedded(value_text, source, line.number, value_column)?;
        if let Some(index) = index {
            if operator != AssignmentOperator::Set {
                return Err(ParseError::new(
                    line.number,
                    line.column,
                    "indexed assignment does not support compound operators",
                ));
            }
            value = Self::indexed_assignment(&name, index, value, source, &synthetic);
        } else {
            value = Self::apply_assignment_operator(&name, value, operator, source, &synthetic);
        }
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

    fn parse_for(
        &mut self,
        line: &Line<'_>,
        rest: &str,
        indent: usize,
    ) -> Result<Stmt, ParseError> {
        let (header, column) = expect_colon_header(line, rest)?;
        let (name, collection) = header.split_once(" in ").ok_or_else(|| {
            ParseError::new(
                line.number,
                line.column,
                "expected `for name in collection:`",
            )
        })?;
        let name = expect_identifier(line, name.trim())?;
        let collection_offset = header.len() - collection.len();
        let collection = parse_embedded(
            collection,
            &self.source,
            line.number,
            column + header[..collection_offset].chars().count(),
        )?;
        let body = self.body(indent, line)?;
        Ok(Stmt::For {
            name,
            collection,
            body,
            line: line.number,
        })
    }

    fn parse_loop_control(line: &Line<'_>, rest: &str, is_break: bool) -> Result<Stmt, ParseError> {
        let keyword = if is_break { "break" } else { "continue" };
        if !rest.is_empty() {
            return Err(ParseError::new(
                line.number,
                line.column,
                format!("`{keyword}` takes no value"),
            ));
        }
        Ok(if is_break {
            Stmt::Break { line: line.number }
        } else {
            Stmt::Continue { line: line.number }
        })
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
