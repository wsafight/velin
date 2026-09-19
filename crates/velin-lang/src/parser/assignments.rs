use super::Parser;
use crate::error::ParseError;
use crate::lines::Line;
use crate::token::{
    AssignmentOperator, AssignmentTarget, expect_identifier, parse_embedded, split_assignment,
    strip_keyword,
};
use velin_parse::parse_expression_list_with_source;
use velin_syntax::{BinaryOp, Builtin, Expr, SharedString, Span};

impl Parser<'_, '_> {
    pub(super) fn parse_bare_assign_or_bind(
        line: &Line<'_>,
        source: &SharedString,
    ) -> Result<crate::ast::Stmt, ParseError> {
        let (target, rhs, rhs_column, operator) = split_assignment(line)?;
        let (name, index) = Self::parse_assignment_target(line, target, source)?;
        if let Some(rest) = strip_keyword(rhs, "call") {
            if operator != AssignmentOperator::Set || index.is_some() {
                return Err(ParseError::new(
                    line.number,
                    line.column,
                    "a function result requires a simple variable assignment",
                ));
            }
            let (module, function, arguments) =
                Self::parse_function_call(line, rest, source, rhs_column)?;
            return Ok(crate::ast::Stmt::Call {
                module,
                function,
                arguments,
                bind: name,
                line: line.number,
            });
        }
        if let Some(rest) = strip_keyword(rhs, "perform") {
            if operator != AssignmentOperator::Set || index.is_some() {
                return Err(ParseError::new(
                    line.number,
                    line.column,
                    "a host result requires a simple variable assignment",
                ));
            }
            let command_line = Line {
                number: line.number,
                indent: line.indent,
                content: rhs,
                column: rhs_column,
            };
            return Self::parse_perform(&command_line, rest, Some(name), source);
        }
        let mut value = parse_embedded(rhs, source, line.number, rhs_column)?;
        if let Some(index) = index {
            if operator != AssignmentOperator::Set {
                return Err(ParseError::new(
                    line.number,
                    line.column,
                    "indexed assignment does not support compound operators",
                ));
            }
            value = Self::indexed_assignment(&name, index, value, source, line);
        } else {
            value = Self::apply_assignment_operator(&name, value, operator, source, line);
        }
        Ok(crate::ast::Stmt::Set {
            name,
            value,
            line: line.number,
        })
    }

    pub(super) fn parse_assignment_target(
        line: &Line<'_>,
        target: AssignmentTarget<'_>,
        source: &SharedString,
    ) -> Result<(String, Option<Expr>), ParseError> {
        let target = match target {
            AssignmentTarget::Identifier(name) => return Ok((name.to_owned(), None)),
            AssignmentTarget::General(target) => target,
        };
        let target = target.trim();
        let Some(open) = target.find('[') else {
            return expect_identifier(line, target).map(|name| (name, None));
        };
        if !target.ends_with(']') {
            return Err(ParseError::new(
                line.number,
                line.column,
                "expected `]` in indexed assignment",
            ));
        }
        let name = expect_identifier(line, target[..open].trim())?;
        let inner = &target[open + 1..target.len() - 1];
        let index = parse_embedded(
            inner,
            source,
            line.number,
            line.column + target[..=open].chars().count(),
        )?;
        Ok((name, Some(index)))
    }

    pub(super) fn apply_assignment_operator(
        name: &str,
        value: Expr,
        operator: AssignmentOperator,
        source: &SharedString,
        line: &Line<'_>,
    ) -> Expr {
        let op = match operator {
            AssignmentOperator::Set => return value,
            AssignmentOperator::Add => BinaryOp::Add,
            AssignmentOperator::Subtract => BinaryOp::Subtract,
            AssignmentOperator::Multiply => BinaryOp::Multiply,
            AssignmentOperator::Divide => BinaryOp::Divide,
        };
        Expr::Binary {
            left: Box::new(Expr::Variable(name.to_owned()).spanned(Span::in_source(
                source.clone(),
                line.number,
                line.column,
            ))),
            op,
            right: Box::new(value),
        }
        .spanned(Span::in_source(source.clone(), line.number, line.column))
    }

    pub(super) fn indexed_assignment(
        name: &str,
        index: Expr,
        value: Expr,
        source: &SharedString,
        line: &Line<'_>,
    ) -> Expr {
        Expr::Invoke {
            function: Builtin::Put,
            arguments: vec![
                Expr::Variable(name.to_owned()).spanned(Span::in_source(
                    source.clone(),
                    line.number,
                    line.column,
                )),
                index,
                value,
            ],
        }
        .spanned(Span::in_source(source.clone(), line.number, line.column))
    }

    pub(super) fn parse_function_call(
        line: &Line<'_>,
        rest: &str,
        source: &SharedString,
        rhs_column: usize,
    ) -> Result<(Option<String>, String, Vec<Expr>), ParseError> {
        let text = rest.trim();
        let open = text.find('(').ok_or_else(|| {
            ParseError::new(line.number, rhs_column, "expected `(` after function name")
        })?;
        if !text.ends_with(')') {
            return Err(ParseError::new(
                line.number,
                rhs_column,
                "expected `)` after function arguments",
            ));
        }
        let qualified = text[..open].trim();
        let (module, function) = match qualified.split_once('.') {
            Some((module, function)) => (
                Some(expect_identifier(line, module.trim())?),
                expect_identifier(line, function.trim())?,
            ),
            None => (None, expect_identifier(line, qualified)?),
        };
        let inner = &text[open + 1..text.len() - 1];
        let arguments = if inner.trim().is_empty() {
            Vec::new()
        } else {
            let text_offset = rest.len() - rest.trim_start().len();
            parse_expression_list_with_source(
                inner,
                source,
                line.number,
                rhs_column + "call ".len() + text_offset + open + 1,
            )
            .map_err(ParseError::from_diagnostic)?
        };
        Ok((module, function, arguments))
    }
}
