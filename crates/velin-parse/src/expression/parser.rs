use super::{
    BinaryOp, Builtin, Diagnostic, Expr, ParseBudget, RawPart, SharedString, Span, StrPart, Token,
    TokenBuffer, TokenKind, UnaryOp, Value, parse_expression_inner,
};

pub(super) struct ExpressionParser<'input, 'file, 'budget> {
    pub(super) tokens: TokenBuffer<'input>,
    pub(super) current: usize,
    pub(super) file: &'file str,
    pub(super) source: SharedString,
    pub(super) line: usize,
    pub(super) interpolation_depth: usize,
    pub(super) budget: &'budget mut ParseBudget,
}

impl<'input> ExpressionParser<'input, '_, '_> {
    pub(super) fn parse_or(&mut self) -> Result<Expr, Diagnostic> {
        let mut expression = self.parse_and()?;
        while let Some(column) = self.consume_column(&TokenKind::Or) {
            let right = self.parse_and()?;
            expression = self.binary(expression, BinaryOp::Or, right, column);
        }
        Ok(expression)
    }

    fn parse_and(&mut self) -> Result<Expr, Diagnostic> {
        let mut expression = self.parse_equality()?;
        while let Some(column) = self.consume_column(&TokenKind::And) {
            let right = self.parse_equality()?;
            expression = self.binary(expression, BinaryOp::And, right, column);
        }
        Ok(expression)
    }

    fn parse_equality(&mut self) -> Result<Expr, Diagnostic> {
        let mut expression = self.parse_comparison()?;
        loop {
            let op = if let Some(column) = self.consume_column(&TokenKind::Equal) {
                Some((BinaryOp::Equal, column))
            } else {
                self.consume_column(&TokenKind::NotEqual)
                    .map(|column| (BinaryOp::NotEqual, column))
            };
            let Some((op, column)) = op else { break };
            let right = self.parse_comparison()?;
            expression = self.binary(expression, op, right, column);
        }
        Ok(expression)
    }

    fn parse_comparison(&mut self) -> Result<Expr, Diagnostic> {
        let mut expression = self.parse_term()?;
        loop {
            let op = if let Some(column) = self.consume_column(&TokenKind::Less) {
                Some((BinaryOp::Less, column))
            } else if let Some(column) = self.consume_column(&TokenKind::LessEqual) {
                Some((BinaryOp::LessEqual, column))
            } else if let Some(column) = self.consume_column(&TokenKind::Greater) {
                Some((BinaryOp::Greater, column))
            } else {
                self.consume_column(&TokenKind::GreaterEqual)
                    .map(|column| (BinaryOp::GreaterEqual, column))
            };
            let Some((op, column)) = op else { break };
            let right = self.parse_term()?;
            expression = self.binary(expression, op, right, column);
        }
        Ok(expression)
    }

    fn parse_term(&mut self) -> Result<Expr, Diagnostic> {
        let mut expression = self.parse_factor()?;
        loop {
            let op = if let Some(column) = self.consume_column(&TokenKind::Plus) {
                Some((BinaryOp::Add, column))
            } else {
                self.consume_column(&TokenKind::Minus)
                    .map(|column| (BinaryOp::Subtract, column))
            };
            let Some((op, column)) = op else { break };
            let right = self.parse_factor()?;
            expression = self.binary(expression, op, right, column);
        }
        Ok(expression)
    }

    fn parse_factor(&mut self) -> Result<Expr, Diagnostic> {
        let mut expression = self.parse_unary()?;
        loop {
            let op = if let Some(column) = self.consume_column(&TokenKind::Star) {
                Some((BinaryOp::Multiply, column))
            } else {
                self.consume_column(&TokenKind::Slash)
                    .map(|column| (BinaryOp::Divide, column))
            };
            let Some((op, column)) = op else { break };
            let right = self.parse_unary()?;
            expression = self.binary(expression, op, right, column);
        }
        Ok(expression)
    }

    fn parse_unary(&mut self) -> Result<Expr, Diagnostic> {
        if let Some(column) = self.consume_column(&TokenKind::Minus) {
            let expression = Expr::Unary {
                op: UnaryOp::Negate,
                value: Box::new(self.parse_unary()?),
            };
            return Ok(expression.spanned(self.span(column)));
        }
        if let Some(column) = self.consume_column(&TokenKind::Not) {
            let expression = Expr::Unary {
                op: UnaryOp::Not,
                value: Box::new(self.parse_unary()?),
            };
            return Ok(expression.spanned(self.span(column)));
        }
        self.parse_postfix()
    }

    fn parse_postfix(&mut self) -> Result<Expr, Diagnostic> {
        let mut expression = self.parse_primary()?;
        while let Some(column) = self.consume_column(&TokenKind::LeftBracket) {
            let index = self.parse_or()?;
            if !self.consume(&TokenKind::RightBracket) {
                return Err(self.error("expected `]` after collection index"));
            }
            expression = Expr::Invoke {
                function: Builtin::Get,
                arguments: vec![expression, index],
            }
            .spanned(self.span(column));
        }
        Ok(expression)
    }

    fn parse_primary(&mut self) -> Result<Expr, Diagnostic> {
        let token = self.advance();
        let span = self.span(token.column);
        match token.kind {
            TokenKind::Integer(value) => Ok(Expr::Value(Value::Integer(value)).spanned(span)),
            TokenKind::Boolean(value) => Ok(Expr::Value(Value::Boolean(value)).spanned(span)),
            TokenKind::String(value) => Ok(Expr::Value(Value::String(value.into())).spanned(span)),
            TokenKind::Interpolated(raw) => Ok(self.interpolate(raw)?.spanned(span)),
            TokenKind::Identifier(value) if self.consume(&TokenKind::LeftParen) => {
                let function = Builtin::named(value)
                    .ok_or_else(|| self.error(format!("unknown built-in function `{value}`")))?;
                let mut arguments = Vec::new();
                if !self.consume(&TokenKind::RightParen) {
                    loop {
                        arguments.push(self.parse_or()?);
                        if self.consume(&TokenKind::RightParen) {
                            break;
                        }
                        if !self.consume(&TokenKind::Comma) {
                            return Err(self.error("expected `,` or `)`"));
                        }
                    }
                }
                if !function.accepts(arguments.len()) {
                    return Err(self.error(format!("invalid argument count for `{value}`")));
                }
                Ok(Expr::Invoke {
                    function,
                    arguments,
                }
                .spanned(span))
            }
            TokenKind::Identifier(value) => Ok(Expr::Variable(value.to_owned()).spanned(span)),
            TokenKind::LeftParen => {
                let expression = self.parse_or()?;
                if !self.consume(&TokenKind::RightParen) {
                    return Err(self.error("expected `)`"));
                }
                Ok(expression)
            }
            TokenKind::LeftBracket => self.parse_list(span),
            TokenKind::LeftBrace => self.parse_record(span),
            _ => Err(Diagnostic::new(
                self.file,
                self.line,
                token.column,
                "expected a value, variable, or parenthesized expression",
            )),
        }
    }

    fn parse_list(&mut self, span: Span) -> Result<Expr, Diagnostic> {
        let arguments = self.parse_delimited(&TokenKind::RightBracket, "]")?;
        if !Builtin::List.accepts(arguments.len()) {
            return Err(self.error("list literal exceeds 128 items"));
        }
        Ok(Expr::Invoke {
            function: Builtin::List,
            arguments,
        }
        .spanned(span))
    }

    fn parse_record(&mut self, span: Span) -> Result<Expr, Diagnostic> {
        let mut arguments = Vec::new();
        if !self.consume(&TokenKind::RightBrace) {
            loop {
                let token = self.advance();
                let key_span = self.span(token.column);
                let key = match token.kind {
                    TokenKind::Identifier(value) => value.to_owned(),
                    TokenKind::String(value) => value,
                    _ => return Err(self.error("expected a record key")),
                };
                if !self.consume(&TokenKind::Colon) {
                    return Err(self.error("expected `:` after record key"));
                }
                arguments.push(Expr::Value(Value::String(key.into())).spanned(key_span));
                arguments.push(self.parse_or()?);
                if self.consume(&TokenKind::RightBrace) {
                    break;
                }
                if !self.consume(&TokenKind::Comma) {
                    return Err(self.error("expected `,` or `}`"));
                }
            }
        }
        if !Builtin::Record.accepts(arguments.len()) {
            return Err(self.error("record literal exceeds 64 entries"));
        }
        Ok(Expr::Invoke {
            function: Builtin::Record,
            arguments,
        }
        .spanned(span))
    }

    fn parse_delimited(
        &mut self,
        closing: &TokenKind<'_>,
        closing_text: &str,
    ) -> Result<Vec<Expr>, Diagnostic> {
        let mut expressions = Vec::new();
        if !self.consume(closing) {
            loop {
                expressions.push(self.parse_or()?);
                if self.consume(closing) {
                    break;
                }
                if !self.consume(&TokenKind::Comma) {
                    return Err(self.error(format!("expected `,` or `{closing_text}`")));
                }
            }
        }
        Ok(expressions)
    }

    /// Builds an [`Expr::Interpolate`] from lexed raw parts, parsing each hole's
    /// captured text as a full sub-expression anchored at its real column.
    fn interpolate(&mut self, raw: Vec<RawPart>) -> Result<Expr, Diagnostic> {
        let mut parts = Vec::with_capacity(raw.len());
        for part in raw {
            match part {
                RawPart::Literal(text) => parts.push(StrPart::Literal(text)),
                RawPart::Hole { text, column } => {
                    let expr = parse_expression_inner(
                        &text,
                        self.file,
                        &self.source,
                        self.line,
                        column,
                        self.interpolation_depth + 1,
                        self.budget,
                    )?;
                    parts.push(StrPart::Hole(Box::new(expr)));
                }
            }
        }
        Ok(Expr::Interpolate { parts })
    }

    pub(super) fn consume(&mut self, kind: &TokenKind<'_>) -> bool {
        if &self.peek().kind == kind {
            self.current += 1;
            true
        } else {
            false
        }
    }

    fn consume_column(&mut self, kind: &TokenKind<'_>) -> Option<usize> {
        if &self.peek().kind != kind {
            return None;
        }
        let column = self.peek().column;
        self.current += 1;
        Some(column)
    }

    fn advance(&mut self) -> Token<'input> {
        let index = self.current;
        if self.tokens.get(index).kind != TokenKind::End {
            self.current += 1;
        }
        self.tokens.take(index)
    }

    pub(super) fn peek(&self) -> &Token<'input> {
        self.tokens.get(self.current)
    }

    pub(super) fn error(&self, message: impl Into<String>) -> Diagnostic {
        Diagnostic::new(self.file, self.line, self.peek().column, message)
    }

    fn span(&self, column: usize) -> Span {
        Span::in_source(self.source.clone(), self.line, column)
    }

    fn binary(&self, left: Expr, op: BinaryOp, right: Expr, column: usize) -> Expr {
        Expr::Binary {
            left: Box::new(left),
            op,
            right: Box::new(right),
        }
        .spanned(self.span(column))
    }
}
