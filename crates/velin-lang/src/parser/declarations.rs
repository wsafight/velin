use super::Parser;
use crate::ast::Stmt;
use crate::error::ParseError;
use crate::lines::Line;
use crate::token::{expect_identifier, parse_embedded, split_keyword};
use velin_syntax::SharedString;

impl Parser<'_, '_> {
    pub(super) fn parse_import(line: &Line<'_>, rest: &str) -> Result<Stmt, ParseError> {
        let name = expect_identifier(line, rest.trim())?;
        Ok(Stmt::Import {
            name,
            line: line.number,
        })
    }

    pub(super) fn parse_export(
        &mut self,
        line: &Line<'_>,
        rest: &str,
        indent: usize,
    ) -> Result<Stmt, ParseError> {
        let (keyword, rest) = split_keyword(rest);
        if keyword != "fn" {
            return Err(ParseError::new(
                line.number,
                line.column,
                "expected `export fn name(...):`",
            ));
        }
        self.parse_function(line, rest, indent, true)
    }

    pub(super) fn parse_function(
        &mut self,
        line: &Line<'_>,
        rest: &str,
        indent: usize,
        exported: bool,
    ) -> Result<Stmt, ParseError> {
        let header = rest.trim().strip_suffix(':').ok_or_else(|| {
            ParseError::new(
                line.number,
                line.column,
                "expected `:` after function signature",
            )
        })?;
        let open = header.find('(').ok_or_else(|| {
            ParseError::new(line.number, line.column, "expected `(` after function name")
        })?;
        if !header.ends_with(')') {
            return Err(ParseError::new(
                line.number,
                line.column,
                "expected `)` after function parameters",
            ));
        }
        let name = expect_identifier(line, header[..open].trim())?;
        let mut parameters = Vec::new();
        for parameter in header[open + 1..header.len() - 1].split(',') {
            let parameter = parameter.trim();
            if parameter.is_empty() {
                continue;
            }
            let parameter = expect_identifier(line, parameter)?;
            if parameters.contains(&parameter) {
                return Err(ParseError::new(
                    line.number,
                    line.column,
                    format!("duplicate parameter `{parameter}`"),
                ));
            }
            parameters.push(parameter);
        }
        let body = self.body(indent, line)?;
        Ok(Stmt::Function {
            name,
            parameters,
            body,
            exported,
            line: line.number,
        })
    }

    pub(super) fn parse_return(
        line: &Line<'_>,
        rest: &str,
        source: &SharedString,
    ) -> Result<Stmt, ParseError> {
        let offset = line.content.len() - rest.len();
        let value = parse_embedded(
            rest,
            source,
            line.number,
            line.column + line.content[..offset].chars().count(),
        )?;
        Ok(Stmt::Return {
            value,
            line: line.number,
        })
    }
}
