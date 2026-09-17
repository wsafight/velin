//! Canonical source formatting built directly on the statement AST.

use crate::{ParseError, Stmt, ast::Branch, parse_program};
use std::fmt::Write as _;
use velin_syntax::{BinaryOp, Builtin, Expr, StrPart, UnaryOp, Value};

const INDENT: &str = "    ";

/// Parses `source` and renders its canonical representation.
///
/// The formatter is intentionally AST based: it accepts every syntax form the
/// compiler accepts, emits four-space indentation and one trailing newline,
/// and is idempotent. Assignment shorthand and collection literals may be
/// expanded to their equivalent core expression forms.
///
/// # Errors
/// Returns the normal parser error when the input is not a valid program.
pub fn format_source(source: &str) -> Result<String, ParseError> {
    let statements = parse_program(source)?;
    let mut output = String::with_capacity(source.len().saturating_add(1));
    format_statements(&statements, 0, &mut output);
    Ok(merge_trivia(source, &output))
}

fn merge_trivia(source: &str, formatted: &str) -> String {
    let mut canonical = formatted.lines();
    let mut output = String::with_capacity(source.len().max(formatted.len()).saturating_add(1));
    let mut pending = Vec::new();
    for raw in source.lines() {
        let trimmed = raw.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            pending.push(raw.trim_end());
            continue;
        }
        for trivia in pending.drain(..) {
            output.push_str(trivia);
            output.push('\n');
        }
        let line = canonical
            .next()
            .expect("valid source and formatted AST have matching statement lines");
        output.push_str(line);
        if let Some(comment) = trailing_comment(raw) {
            output.push_str("  ");
            output.push_str(comment.trim_start());
        }
        output.push('\n');
    }
    for trivia in pending {
        output.push_str(trivia);
        output.push('\n');
    }
    debug_assert!(canonical.next().is_none());
    output
}

fn trailing_comment(line: &str) -> Option<&str> {
    let mut in_string = false;
    let mut escaped = false;
    let mut interpolation_depth = 0usize;
    let mut interpolation_string = false;
    let mut interpolation_escaped = false;
    let mut characters = line.char_indices().peekable();
    while let Some((index, character)) = characters.next() {
        if in_string {
            if interpolation_depth > 0 {
                if interpolation_string {
                    if interpolation_escaped {
                        interpolation_escaped = false;
                    } else if character == '\\' {
                        interpolation_escaped = true;
                    } else if character == '"' {
                        interpolation_string = false;
                    }
                } else {
                    match character {
                        '"' => interpolation_string = true,
                        '[' => interpolation_depth += 1,
                        ']' => interpolation_depth -= 1,
                        _ => {}
                    }
                }
            } else if escaped {
                escaped = false;
            } else if character == '\\' {
                escaped = true;
            } else if character == '"' {
                in_string = false;
            } else if character == '[' {
                if characters.peek().is_some_and(|(_, next)| *next == '[') {
                    characters.next();
                } else {
                    interpolation_depth = 1;
                }
            }
        } else if character == '"' {
            in_string = true;
        } else if character == '#' {
            return Some(&line[index..]);
        }
    }
    None
}

fn format_statements(statements: &[Stmt], depth: usize, output: &mut String) {
    for statement in statements {
        format_statement(statement, depth, output);
    }
}

#[allow(clippy::too_many_lines)]
fn format_statement(statement: &Stmt, depth: usize, output: &mut String) {
    let indent = INDENT.repeat(depth);
    output.push_str(&indent);
    match statement {
        Stmt::Import { name, .. } => writeln!(output, "import {name}"),
        Stmt::Function {
            name,
            parameters,
            body,
            exported,
            ..
        } => {
            if *exported {
                output.push_str("export ");
            }
            write!(output, "fn {name}(").expect("string writes cannot fail");
            write_joined(parameters, output, |parameter, output| {
                output.push_str(parameter);
            });
            output.push_str("):\n");
            format_statements(body, depth + 1, output);
            Ok(())
        }
        Stmt::Call {
            module,
            function,
            arguments,
            bind,
            ..
        } => {
            write!(output, "{bind} = call ").expect("string writes cannot fail");
            if let Some(module) = module {
                write!(output, "{module}.").expect("string writes cannot fail");
            }
            write!(output, "{function}(").expect("string writes cannot fail");
            format_arguments(arguments, output);
            output.push_str(")\n");
            Ok(())
        }
        Stmt::Return { value, .. } => {
            output.push_str("return ");
            format_expression(value, 0, output);
            output.push('\n');
            Ok(())
        }
        Stmt::Label { name, body, .. } => {
            writeln!(output, "label {name}:").expect("string writes cannot fail");
            format_statements(body, depth + 1, output);
            Ok(())
        }
        Stmt::Default { name, value, .. } => {
            write!(output, "default {name} = ").expect("string writes cannot fail");
            format_expression(value, 0, output);
            output.push('\n');
            Ok(())
        }
        Stmt::Set { name, value, .. } => {
            write!(output, "set {name} = ").expect("string writes cannot fail");
            format_expression(value, 0, output);
            output.push('\n');
            Ok(())
        }
        Stmt::Perform {
            command,
            arguments,
            bind,
            ..
        } => {
            if let Some(bind) = bind {
                write!(output, "{bind} = ").expect("string writes cannot fail");
            }
            write!(output, "perform {command}(").expect("string writes cannot fail");
            format_arguments(arguments, output);
            output.push_str(")\n");
            Ok(())
        }
        Stmt::If {
            branches,
            otherwise,
        } => {
            for (index, branch) in branches.iter().enumerate() {
                if index > 0 {
                    output.push_str(&indent);
                }
                format_branch(
                    if index == 0 { "if" } else { "elif" },
                    branch,
                    depth,
                    output,
                );
            }
            if let Some(body) = otherwise {
                output.push_str(&indent);
                output.push_str("else:\n");
                format_statements(body, depth + 1, output);
            }
            Ok(())
        }
        Stmt::While {
            condition, body, ..
        } => {
            output.push_str("while ");
            format_expression(&condition.expr, 0, output);
            output.push_str(":\n");
            format_statements(body, depth + 1, output);
            Ok(())
        }
        Stmt::For {
            name,
            collection,
            body,
            ..
        } => {
            write!(output, "for {name} in ").expect("string writes cannot fail");
            format_expression(collection, 0, output);
            output.push_str(":\n");
            format_statements(body, depth + 1, output);
            Ok(())
        }
        Stmt::Break { .. } => writeln!(output, "break"),
        Stmt::Continue { .. } => writeln!(output, "continue"),
        Stmt::Jump { label, .. } => writeln!(output, "jump {label}"),
    }
    .expect("string writes cannot fail");
}

fn format_branch(keyword: &str, branch: &Branch, depth: usize, output: &mut String) {
    write!(output, "{keyword} ").expect("string writes cannot fail");
    format_expression(&branch.condition.expr, 0, output);
    output.push_str(":\n");
    format_statements(&branch.body, depth + 1, output);
}

fn format_arguments(arguments: &[Expr], output: &mut String) {
    write_joined(arguments, output, |argument, output| {
        format_expression(argument, 0, output);
    });
}

fn write_joined<T>(items: &[T], output: &mut String, mut write_item: impl FnMut(&T, &mut String)) {
    for (index, item) in items.iter().enumerate() {
        if index > 0 {
            output.push_str(", ");
        }
        write_item(item, output);
    }
}

fn format_expression(expression: &Expr, parent_precedence: u8, output: &mut String) {
    let expression = expression.unspanned();
    let precedence = precedence(expression);
    let parenthesized = precedence < parent_precedence;
    if parenthesized {
        output.push('(');
    }
    match expression {
        Expr::Spanned { .. } => unreachable!("unspanned removes wrappers"),
        Expr::Value(value) => format_value(value, output),
        Expr::Variable(name) => output.push_str(name),
        Expr::Invoke {
            function,
            arguments,
        } => {
            output.push_str(builtin_name(*function));
            output.push('(');
            format_arguments(arguments, output);
            output.push(')');
        }
        Expr::Unary { op, value } => {
            output.push_str(match op {
                UnaryOp::Negate => "-",
                UnaryOp::Not => "not ",
            });
            format_expression(value, precedence, output);
        }
        Expr::Binary { left, op, right } => {
            format_expression(left, precedence, output);
            write!(output, " {} ", binary_name(*op)).expect("string writes cannot fail");
            format_expression(right, precedence.saturating_add(1), output);
        }
        Expr::Interpolate { parts } => format_interpolation(parts, output),
    }
    if parenthesized {
        output.push(')');
    }
}

const fn precedence(expression: &Expr) -> u8 {
    match expression {
        Expr::Spanned { .. } => 0,
        Expr::Binary { op, .. } => match op {
            BinaryOp::Or => 1,
            BinaryOp::And => 2,
            BinaryOp::Equal | BinaryOp::NotEqual => 3,
            BinaryOp::Less | BinaryOp::LessEqual | BinaryOp::Greater | BinaryOp::GreaterEqual => 4,
            BinaryOp::Add | BinaryOp::Subtract => 5,
            BinaryOp::Multiply | BinaryOp::Divide => 6,
        },
        Expr::Unary { .. } => 7,
        Expr::Invoke { .. } | Expr::Value(_) | Expr::Variable(_) | Expr::Interpolate { .. } => 8,
    }
}

fn format_value(value: &Value, output: &mut String) {
    match value {
        Value::Integer(value) => write!(output, "{value}").expect("string writes cannot fail"),
        Value::Boolean(value) => output.push_str(if *value { "true" } else { "false" }),
        Value::String(value) => format_string(value, output),
        Value::List(values) => {
            output.push_str("list(");
            write_joined(values, output, format_value);
            output.push(')');
        }
        Value::Record(fields) => {
            output.push_str("record(");
            for (index, (name, value)) in fields.iter().enumerate() {
                if index > 0 {
                    output.push_str(", ");
                }
                format_string(name, output);
                output.push_str(", ");
                format_value(value, output);
            }
            output.push(')');
        }
    }
}

fn format_interpolation(parts: &[StrPart], output: &mut String) {
    output.push('"');
    for part in parts {
        match part {
            StrPart::Literal(text) => escape_string(text, true, output),
            StrPart::Hole(expression) => {
                output.push('[');
                format_expression(expression, 0, output);
                output.push(']');
            }
        }
    }
    output.push('"');
}

fn format_string(value: &str, output: &mut String) {
    output.push('"');
    escape_string(value, false, output);
    output.push('"');
}

fn escape_string(value: &str, interpolation: bool, output: &mut String) {
    for character in value.chars() {
        match character {
            '\\' => output.push_str("\\\\"),
            '"' => output.push_str("\\\""),
            '\n' => output.push_str("\\n"),
            '\r' => output.push_str("\\r"),
            '\t' => output.push_str("\\t"),
            '[' if interpolation => output.push_str("[["),
            ']' if interpolation => output.push_str("]]"),
            other => output.push(other),
        }
    }
}

const fn builtin_name(builtin: Builtin) -> &'static str {
    match builtin {
        Builtin::List => "list",
        Builtin::Record => "record",
        Builtin::Get => "get",
        Builtin::Put => "put",
        Builtin::Push => "push",
        Builtin::Remove => "remove",
        Builtin::Len => "len",
        Builtin::Contains => "contains",
        Builtin::Random => "random",
        Builtin::Chance => "chance",
    }
}

const fn binary_name(operator: BinaryOp) -> &'static str {
    match operator {
        BinaryOp::Add => "+",
        BinaryOp::Subtract => "-",
        BinaryOp::Multiply => "*",
        BinaryOp::Divide => "/",
        BinaryOp::Equal => "==",
        BinaryOp::NotEqual => "!=",
        BinaryOp::Less => "<",
        BinaryOp::LessEqual => "<=",
        BinaryOp::Greater => ">",
        BinaryOp::GreaterEqual => ">=",
        BinaryOp::And => "and",
        BinaryOp::Or => "or",
    }
}

#[cfg(test)]
mod tests {
    use super::format_source;

    #[test]
    fn formatting_is_canonical_and_idempotent() {
        let source = "default items=[1,2]\nif true and (false or true):\n    items[0]=2\nelse:\n    perform say(\"value [items[0]]\")\n";
        let formatted = format_source(source).unwrap();
        assert!(formatted.contains("default items = list(1, 2)"));
        assert!(formatted.contains("if true and (false or true):"));
        assert!(formatted.contains("set items = put(items, 0, 2)"));
        assert_eq!(format_source(&formatted).unwrap(), formatted);
    }

    #[test]
    fn formatter_rejects_invalid_source() {
        assert!(format_source("if true\n").is_err());
    }

    #[test]
    fn formatter_preserves_comments_and_blank_lines() {
        let source = "# heading\n\nset text=\"# [1]\" # result\n    # nested note\nif true:\n    perform say(text)\n";
        let formatted = format_source(source).unwrap();
        assert_eq!(
            formatted,
            "# heading\n\nset text = \"# [1]\"  # result\n    # nested note\nif true:\n    perform say(text)\n"
        );
        assert_eq!(format_source(&formatted).unwrap(), formatted);
    }
}
