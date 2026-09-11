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
use velin_syntax::Expr;

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
    };
    let mut errors = Vec::new();
    let statements = parser.recovering_block(0, &mut errors);
    RecoveredProgram { statements, errors }
}

struct Parser<'a> {
    lines: &'a [Line],
    at: usize,
}

impl<'a> Parser<'a> {
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

    /// Best-effort counterpart to [`Self::block`] used only by editor tooling.
    fn recovering_block(&mut self, indent: usize, errors: &mut Vec<ParseError>) -> Vec<Stmt> {
        if indent > MAX_STATEMENT_DEPTH {
            let line = self.peek().expect("nested block starts with a line");
            errors.push(ParseError::new(
                line.number,
                line.column,
                format!("statement nesting exceeds {MAX_STATEMENT_DEPTH}"),
            ));
            while self.peek().is_some_and(|line| line.indent >= indent) {
                self.advance();
            }
            return Vec::new();
        }

        let mut statements = Vec::new();
        while let Some(line) = self.peek() {
            if line.indent < indent {
                break;
            }
            if line.indent > indent {
                errors.push(ParseError::new(
                    line.number,
                    line.column,
                    "unexpected indentation",
                ));
                self.skip_nested(indent);
                continue;
            }
            if let Some(statement) = self.recovering_statement(indent, errors) {
                statements.push(statement);
            }
        }
        statements
    }

    fn recovering_statement(
        &mut self,
        indent: usize,
        errors: &mut Vec<ParseError>,
    ) -> Option<Stmt> {
        let line = self
            .advance()
            .expect("statement called with a line present");
        let (keyword, rest) = split_keyword(&line.content);
        match keyword {
            "label" => self.recovering_label(line, rest, indent, errors),
            "if" => self.recovering_if(line, rest, indent, errors),
            "while" => self.recovering_while(line, rest, indent, errors),
            "default" => self.recover_simple(Self::parse_binding(line, rest, true), indent, errors),
            "set" => self.recover_simple(Self::parse_binding(line, rest, false), indent, errors),
            "perform" => self.recover_simple(Self::parse_perform(line, rest, None), indent, errors),
            "jump" => self.recover_simple(Self::parse_jump(line, rest), indent, errors),
            _ => self.recover_simple(Self::parse_bare_assign_or_bind(line), indent, errors),
        }
    }

    fn recover_simple(
        &mut self,
        result: Result<Stmt, ParseError>,
        indent: usize,
        errors: &mut Vec<ParseError>,
    ) -> Option<Stmt> {
        match result {
            Ok(statement) => Some(statement),
            Err(error) => {
                errors.push(error);
                self.skip_nested(indent);
                None
            }
        }
    }

    fn recovering_label(
        &mut self,
        line: &Line,
        rest: &str,
        indent: usize,
        errors: &mut Vec<ParseError>,
    ) -> Option<Stmt> {
        let name = match expect_header_name(line, rest) {
            Ok(name) => name,
            Err(error) => {
                errors.push(error);
                self.skip_nested(indent);
                return None;
            }
        };
        let body = match self.peek() {
            Some(next) if next.indent == indent + 1 => self.recovering_block(indent + 1, errors),
            _ => Vec::new(),
        };
        Some(Stmt::Label {
            name,
            body,
            line: line.number,
        })
    }

    fn recovering_if(
        &mut self,
        line: &Line,
        rest: &str,
        indent: usize,
        errors: &mut Vec<ParseError>,
    ) -> Option<Stmt> {
        let mut branches = Vec::new();
        if let Some(branch) = self.recovering_branch(line, rest, indent, errors) {
            branches.push(branch);
        }
        let mut otherwise = None;

        while let Some(next) = self.peek() {
            if next.indent != indent {
                break;
            }
            let (keyword, rest) = split_keyword(&next.content);
            match keyword {
                "elif" => {
                    let header = self.advance().expect("peeked line present");
                    if let Some(branch) = self.recovering_branch(header, rest, indent, errors) {
                        branches.push(branch);
                    }
                }
                "else" => {
                    let header = self.advance().expect("peeked line present");
                    match expect_bare_header(header, rest, "else") {
                        Ok(()) => {
                            otherwise = self.recovering_body(indent, header, errors);
                        }
                        Err(error) => {
                            errors.push(error);
                            self.skip_nested(indent);
                        }
                    }
                    break;
                }
                _ => break,
            }
        }

        if branches.is_empty() && otherwise.is_none() {
            None
        } else {
            Some(Stmt::If {
                branches,
                otherwise,
            })
        }
    }

    fn recovering_branch(
        &mut self,
        header: &Line,
        rest: &str,
        indent: usize,
        errors: &mut Vec<ParseError>,
    ) -> Option<Branch> {
        let condition = match Self::parse_condition(header, rest) {
            Ok(condition) => condition,
            Err(error) => {
                errors.push(error);
                self.skip_nested(indent);
                return None;
            }
        };
        let body = self.recovering_body(indent, header, errors)?;
        Some(Branch { condition, body })
    }

    fn recovering_while(
        &mut self,
        line: &Line,
        rest: &str,
        indent: usize,
        errors: &mut Vec<ParseError>,
    ) -> Option<Stmt> {
        let condition = match Self::parse_condition(line, rest) {
            Ok(condition) => condition,
            Err(error) => {
                errors.push(error);
                self.skip_nested(indent);
                return None;
            }
        };
        let body = self.recovering_body(indent, line, errors)?;
        Some(Stmt::While { condition, body })
    }

    fn recovering_body(
        &mut self,
        indent: usize,
        header: &Line,
        errors: &mut Vec<ParseError>,
    ) -> Option<Vec<Stmt>> {
        match self.peek() {
            Some(next) if next.indent == indent + 1 => {
                Some(self.recovering_block(indent + 1, errors))
            }
            _ => {
                errors.push(ParseError::new(
                    header.number,
                    header.column,
                    "expected an indented block after this line",
                ));
                self.skip_nested(indent);
                None
            }
        }
    }

    fn skip_nested(&mut self, indent: usize) {
        while self.peek().is_some_and(|line| line.indent > indent) {
            self.advance();
        }
    }

    /// Parses one statement (and, for compound statements, its nested body).
    fn statement(&mut self, indent: usize) -> Result<Stmt, ParseError> {
        let line = self
            .advance()
            .expect("statement called with a line present");
        let (keyword, rest) = split_keyword(&line.content);
        match keyword {
            "label" => self.parse_label(line, rest, indent),
            "default" => Self::parse_binding(line, rest, true),
            "set" => Self::parse_binding(line, rest, false),
            "perform" => Self::parse_perform(line, rest, None),
            "if" => self.parse_if(line, rest, indent),
            "while" => self.parse_while(line, rest, indent),
            "jump" => Self::parse_jump(line, rest),
            _ => Self::parse_bare_assign_or_bind(line),
        }
    }

    fn parse_label(&mut self, line: &Line, rest: &str, indent: usize) -> Result<Stmt, ParseError> {
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
    fn parse_binding(line: &Line, rest: &str, is_default: bool) -> Result<Stmt, ParseError> {
        let (name, value) = Self::split_assignment(line, rest)?;
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
    fn parse_perform(line: &Line, rest: &str, bind: Option<String>) -> Result<Stmt, ParseError> {
        let (command, arguments) = Self::parse_call(line, rest)?;
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
    fn parse_bare_assign_or_bind(line: &Line) -> Result<Stmt, ParseError> {
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
                content: rhs.to_owned(),
                column: rhs_column,
            };
            return Self::parse_perform(&command_line, rest, Some(name));
        }
        let value = parse_embedded(rhs, line.number, rhs_column)?;
        Ok(Stmt::Set {
            name,
            value,
            line: line.number,
        })
    }

    fn parse_if(&mut self, line: &Line, rest: &str, indent: usize) -> Result<Stmt, ParseError> {
        let mut branches = vec![self.parse_branch(line, rest, indent)?];
        let mut otherwise = None;
        while let Some(next) = self.peek() {
            if next.indent != indent {
                break;
            }
            let (keyword, rest) = split_keyword(&next.content);
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
        header: &Line,
        rest: &str,
        indent: usize,
    ) -> Result<Branch, ParseError> {
        let condition = Self::parse_condition(header, rest)?;
        let body = self.body(indent, header)?;
        Ok(Branch { condition, body })
    }

    fn parse_while(&mut self, line: &Line, rest: &str, indent: usize) -> Result<Stmt, ParseError> {
        let condition = Self::parse_condition(line, rest)?;
        let body = self.body(indent, line)?;
        Ok(Stmt::While { condition, body })
    }

    fn parse_jump(line: &Line, rest: &str) -> Result<Stmt, ParseError> {
        let label = expect_identifier(line, rest.trim())?;
        Ok(Stmt::Jump {
            label,
            line: line.number,
        })
    }

    /// Parses the `<expr>:` guard shared by `if`/`elif`/`while`.
    fn parse_condition(line: &Line, rest: &str) -> Result<Condition, ParseError> {
        let (expr_text, expr_column) = expect_colon_header(line, rest)?;
        let expr = parse_embedded(expr_text, line.number, expr_column)?;
        Ok(Condition {
            expr,
            line: line.number,
        })
    }

    /// Parses the indented body block that must follow a compound-statement
    /// header, erroring if none is present.
    fn body(&mut self, indent: usize, header: &Line) -> Result<Vec<Stmt>, ParseError> {
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
    fn split_assignment(line: &Line, rest: &str) -> Result<(String, Expr), ParseError> {
        let rest_offset = line.content.len() - rest.len();
        let synthetic = Line {
            number: line.number,
            indent: line.indent,
            content: rest.to_owned(),
            // `rest` starts one keyword+space past the content column.
            column: line.column + line.content[..rest_offset].chars().count(),
        };
        let (name, value_text, value_column) = split_eq(&synthetic)?;
        let value = parse_embedded(value_text, line.number, value_column)?;
        Ok((name, value))
    }

    /// Parses `cmd(arg, arg, ...)` into a command name and argument expressions.
    fn parse_call(line: &Line, rest: &str) -> Result<(String, Vec<Expr>), ParseError> {
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
            // Wrap the argument list as a call to a known builtin so the
            // expression grammar splits arguments and validates nesting; then
            // unwrap. `list(...)` accepts up to 128 args, matching our budget.
            let wrapped = format!("list({inner})");
            let rest_offset = line.content.len() - rest.len();
            let text_offset = rest.len() - rest.trim_start().len();
            let inner_offset = rest_offset + text_offset + open + 1;
            let inner_column = line.column + line.content[..inner_offset].chars().count();
            let wrapper_column = inner_column - "list(".chars().count();
            match parse_embedded(&wrapped, line.number, wrapper_column)?.into_unspanned() {
                Expr::Invoke { arguments, .. } => arguments,
                _ => unreachable!("wrapped list always parses to an Invoke"),
            }
        };
        Ok((command, arguments))
    }

    fn peek(&self) -> Option<&'a Line> {
        self.lines.get(self.at)
    }

    fn advance(&mut self) -> Option<&'a Line> {
        let line = self.lines.get(self.at);
        if line.is_some() {
            self.at += 1;
        }
        line
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use velin_syntax::{BinaryOp, Value};

    #[test]
    fn parses_label_default_and_set() {
        let stmts = parse("label start:\ndefault hp = 30\nset hp = hp + 10\n").unwrap();
        assert_eq!(stmts.len(), 3);
        assert!(matches!(stmts[0], Stmt::Label { .. }));
        assert!(matches!(stmts[1], Stmt::Default { .. }));
        match &stmts[2] {
            Stmt::Set { name, value, .. } => {
                assert_eq!(name, "hp");
                assert!(matches!(
                    value.unspanned(),
                    Expr::Binary {
                        op: BinaryOp::Add,
                        ..
                    }
                ));
            }
            other => panic!("expected Set, got {other:?}"),
        }
    }

    #[test]
    fn parses_perform_bound_and_unbound() {
        let stmts = parse("perform say(\"hi\")\nchoice = perform ask(\"go?\")\n").unwrap();
        match &stmts[0] {
            Stmt::Perform {
                command,
                arguments,
                bind,
                ..
            } => {
                assert_eq!(command, "say");
                assert_eq!(arguments.len(), 1);
                assert!(bind.is_none());
            }
            other => panic!("expected Perform, got {other:?}"),
        }
        match &stmts[1] {
            Stmt::Perform { command, bind, .. } => {
                assert_eq!(command, "ask");
                assert_eq!(bind.as_deref(), Some("choice"));
            }
            other => panic!("expected bound Perform, got {other:?}"),
        }
    }

    #[test]
    fn parses_if_elif_else_with_bodies() {
        let src = "if hp > 20:\n\
                   \x20\x20\x20\x20set ok = true\n\
                   elif hp > 0:\n\
                   \x20\x20\x20\x20set ok = false\n\
                   else:\n\
                   \x20\x20\x20\x20jump dead\n";
        let stmts = parse(src).unwrap();
        assert_eq!(stmts.len(), 1);
        match &stmts[0] {
            Stmt::If {
                branches,
                otherwise,
            } => {
                assert_eq!(branches.len(), 2);
                assert_eq!(branches[0].body.len(), 1);
                assert!(otherwise.is_some());
            }
            other => panic!("expected If, got {other:?}"),
        }
    }

    #[test]
    fn parses_while_and_nested_block() {
        let src = "while count < 3:\n\
                   \x20\x20\x20\x20set count = count + 1\n\
                   \x20\x20\x20\x20perform tick()\n";
        let stmts = parse(src).unwrap();
        match &stmts[0] {
            Stmt::While { body, .. } => assert_eq!(body.len(), 2),
            other => panic!("expected While, got {other:?}"),
        }
    }

    #[test]
    fn expression_errors_report_source_character_columns() {
        let ascii = parse("set x = \"a\" +\n").unwrap_err();
        let unicode = parse("set x = \"\u{4f60}\" +\n").unwrap_err();
        let unicode_spacing = parse("set\u{a0}x\u{a0}= \"a\" +\n").unwrap_err();
        let host_argument = parse("perform say(\"a\", 1 + )\n").unwrap_err();

        assert_eq!(ascii.column, 14);
        assert_eq!(unicode.column, 14);
        assert_eq!(unicode_spacing.column, 14);
        assert_eq!(host_argument.column, 22);
    }

    #[test]
    fn bare_assignment_without_set_keyword_is_sugar() {
        let stmts = parse("score = 1\n").unwrap();
        match &stmts[0] {
            Stmt::Set { name, value, .. } => {
                assert_eq!(name, "score");
                assert_eq!(value.unspanned(), &Expr::Value(Value::Integer(1)));
            }
            other => panic!("expected Set, got {other:?}"),
        }
    }

    #[test]
    fn missing_block_and_bad_indent_are_errors() {
        assert!(parse("if hp > 0:\nset x = 1\n").is_err()); // body not indented
        assert!(parse("\x20\x20\x20\x20set x = 1\n").is_err()); // leading indent with no header
        assert!(parse("label 1bad:\n").is_err()); // invalid identifier
    }

    #[test]
    fn statement_nesting_is_bounded() {
        let mut source = String::new();
        for depth in 0..=MAX_STATEMENT_DEPTH {
            source.push_str(&"    ".repeat(depth));
            source.push_str("while true:\n");
        }
        source.push_str(&"    ".repeat(MAX_STATEMENT_DEPTH + 1));
        source.push_str("set x = 1\n");
        let error = parse(&source).unwrap_err();
        assert!(error.message.contains("nesting"));
    }

    #[test]
    fn equals_in_condition_is_not_an_assignment() {
        // `set` value contains `==`; the top-level `=` splitter must not trip.
        let stmts = parse("set ok = hp == 3\n").unwrap();
        match &stmts[0] {
            Stmt::Set { value, .. } => assert!(matches!(
                value.unspanned(),
                Expr::Binary {
                    op: BinaryOp::Equal,
                    ..
                }
            )),
            other => panic!("expected Set, got {other:?}"),
        }
    }

    #[test]
    fn perform_jump_label_body_and_malformed_headers() {
        let stmts = parse("label start:\n    perform tick()\n    jump start\n").unwrap();
        assert_eq!(stmts.len(), 1);
        match &stmts[0] {
            Stmt::Label { name, body, .. } => {
                assert_eq!(name, "start");
                assert_eq!(body.len(), 2);
            }
            other => panic!("expected Label, got {other:?}"),
        }

        assert!(parse("perform say\n").unwrap_err().message.contains("`(`"));
        assert!(
            parse("perform say(\"hi\"\n")
                .unwrap_err()
                .message
                .contains("`)`")
        );
        assert!(parse("set x\n").is_err());
        assert!(parse("label start\n").unwrap_err().message.contains("`:`"));
        assert!(
            parse("if true:\n    set x = 1\nelse x:\n    set y = 1\n")
                .unwrap_err()
                .message
                .contains("takes no condition")
        );
        assert!(parse("jump\n").is_err());
    }

    #[test]
    fn recovering_parse_keeps_valid_siblings_and_collects_errors() {
        let recovered = parse_recovering(
            "default before = 1\n\
             set broken\n\
             perform missing(\n\
             label after:\n",
        );

        assert_eq!(recovered.errors.len(), 2);
        assert_eq!(recovered.errors[0].line, 2);
        assert_eq!(recovered.errors[1].line, 3);
        assert_eq!(recovered.statements.len(), 2);
        assert!(matches!(
            &recovered.statements[0],
            Stmt::Default { name, .. } if name == "before"
        ));
        assert!(matches!(
            &recovered.statements[1],
            Stmt::Label { name, .. } if name == "after"
        ));
    }

    #[test]
    fn recovering_parse_retains_valid_statements_inside_a_damaged_block() {
        let recovered = parse_recovering(
            "label start:\n\
             \x20\x20\x20\x20set before = 1\n\
             \x20\x20\x20\x20set broken\n\
             \x20\x20\x20\x20set after = 2\n\
             label done:\n",
        );

        assert_eq!(recovered.errors.len(), 1);
        assert_eq!(recovered.errors[0].line, 3);
        assert_eq!(recovered.statements.len(), 2);
        let Stmt::Label { body, .. } = &recovered.statements[0] else {
            panic!("expected recovered label");
        };
        assert_eq!(body.len(), 2);
        assert!(matches!(&body[0], Stmt::Set { name, .. } if name == "before"));
        assert!(matches!(&body[1], Stmt::Set { name, .. } if name == "after"));
    }

    #[test]
    fn recovering_parse_resynchronizes_after_a_missing_block() {
        let recovered = parse_recovering("while true:\nlabel done:\n");

        assert_eq!(recovered.errors.len(), 1);
        assert_eq!(recovered.errors[0].line, 1);
        assert!(matches!(
            recovered.statements.as_slice(),
            [Stmt::Label { name, .. }] if name == "done"
        ));
    }
}
