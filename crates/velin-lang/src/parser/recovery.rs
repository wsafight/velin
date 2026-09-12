use super::{
    Branch, Line, MAX_STATEMENT_DEPTH, ParseError, Parser, Stmt, expect_bare_header,
    expect_header_name, split_keyword,
};

impl Parser<'_, '_> {
    /// Best-effort counterpart to [`Self::block`] used only by editor tooling.
    pub(super) fn recovering_block(
        &mut self,
        indent: usize,
        errors: &mut Vec<ParseError>,
    ) -> Vec<Stmt> {
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
        let (keyword, rest) = split_keyword(line.content);
        match keyword {
            "label" => self.recovering_label(line, rest, indent, errors),
            "if" => self.recovering_if(line, rest, indent, errors),
            "while" => self.recovering_while(line, rest, indent, errors),
            "default" => {
                let result = Self::parse_binding(line, rest, true, &self.source);
                self.recover_simple(result, indent, errors)
            }
            "set" => {
                let result = Self::parse_binding(line, rest, false, &self.source);
                self.recover_simple(result, indent, errors)
            }
            "perform" => {
                let result = Self::parse_perform(line, rest, None, &self.source);
                self.recover_simple(result, indent, errors)
            }
            "jump" => self.recover_simple(Self::parse_jump(line, rest), indent, errors),
            _ => {
                let result = Self::parse_bare_assign_or_bind(line, &self.source);
                self.recover_simple(result, indent, errors)
            }
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
        line: &Line<'_>,
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
        line: &Line<'_>,
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
            let (keyword, rest) = split_keyword(next.content);
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
        header: &Line<'_>,
        rest: &str,
        indent: usize,
        errors: &mut Vec<ParseError>,
    ) -> Option<Branch> {
        let condition = match Self::parse_condition(header, rest, &self.source) {
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
        line: &Line<'_>,
        rest: &str,
        indent: usize,
        errors: &mut Vec<ParseError>,
    ) -> Option<Stmt> {
        let condition = match Self::parse_condition(line, rest, &self.source) {
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
        header: &Line<'_>,
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
}
