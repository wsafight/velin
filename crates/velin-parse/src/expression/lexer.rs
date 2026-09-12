use super::{Diagnostic, MAX_EXPRESSION_NESTING, RawPart, Token, TokenBuffer, TokenKind};

pub(super) struct Lexer<'input, 'file> {
    input: &'input str,
    file: &'file str,
    line: usize,
    offset: usize,
    current_column: usize,
}

impl<'input, 'file> Lexer<'input, 'file> {
    pub(super) const fn new(
        input: &'input str,
        file: &'file str,
        line: usize,
        base_column: usize,
    ) -> Self {
        Self {
            input,
            file,
            line,
            offset: 0,
            current_column: base_column,
        }
    }

    #[allow(clippy::too_many_lines)]
    pub(super) fn lex(mut self) -> Result<TokenBuffer<'input>, Diagnostic> {
        let mut tokens = TokenBuffer::new(self.input.len());
        let mut depth = 0usize;
        while let Some(ch) = self.peek_char() {
            if ch.is_whitespace() {
                self.bump();
                continue;
            }

            let column = self.column();
            let kind = match ch {
                '(' => {
                    self.bump();
                    TokenKind::LeftParen
                }
                ')' => {
                    self.bump();
                    TokenKind::RightParen
                }
                ',' => {
                    self.bump();
                    TokenKind::Comma
                }
                '+' => {
                    self.bump();
                    TokenKind::Plus
                }
                '-' => {
                    self.bump();
                    TokenKind::Minus
                }
                '*' => {
                    self.bump();
                    TokenKind::Star
                }
                '/' => {
                    self.bump();
                    TokenKind::Slash
                }
                '=' => {
                    self.bump();
                    if self.peek_char() == Some('=') {
                        self.bump();
                        TokenKind::Equal
                    } else {
                        return Err(self.error_at(column, "expected `==` in expression"));
                    }
                }
                '!' => {
                    self.bump();
                    if self.peek_char() == Some('=') {
                        self.bump();
                        TokenKind::NotEqual
                    } else {
                        return Err(self.error_at(column, "expected `!=` in expression"));
                    }
                }
                '<' => {
                    self.bump();
                    if self.peek_char() == Some('=') {
                        self.bump();
                        TokenKind::LessEqual
                    } else {
                        TokenKind::Less
                    }
                }
                '>' => {
                    self.bump();
                    if self.peek_char() == Some('=') {
                        self.bump();
                        TokenKind::GreaterEqual
                    } else {
                        TokenKind::Greater
                    }
                }
                '"' => self.string()?,
                c if c.is_ascii_digit() => TokenKind::Integer(self.integer()?),
                c if is_identifier_start(c) => {
                    let word = self.identifier();
                    match word {
                        "true" => TokenKind::Boolean(true),
                        "false" => TokenKind::Boolean(false),
                        "and" => TokenKind::And,
                        "or" => TokenKind::Or,
                        "not" => TokenKind::Not,
                        _ => TokenKind::Identifier(word),
                    }
                }
                _ => {
                    return Err(self.error_at(column, format!("unexpected character `{ch}`")));
                }
            };
            match &kind {
                TokenKind::LeftParen => {
                    depth += 1;
                    if depth > MAX_EXPRESSION_NESTING {
                        return Err(self.error_at(column, "expression nesting exceeds 32"));
                    }
                }
                TokenKind::RightParen => depth = depth.saturating_sub(1),
                _ => {}
            }
            tokens.push(Token { kind, column });
        }
        tokens.push(Token {
            kind: TokenKind::End,
            column: self.column(),
        });
        Ok(tokens)
    }

    /// Lexes a `"..."` string literal, recognising `[expr]` interpolation holes
    /// (with `[[` escaping a literal `[`).
    ///
    /// Returns a plain [`TokenKind::String`] when there are no holes so nothing
    /// downstream regresses, or a [`TokenKind::Interpolated`] carrying literal
    /// segments and the raw source text + column of each hole. String escapes
    /// (`\n`, `\"`, ...) are honoured inside literal segments; hole text is kept
    /// verbatim so the sub-expression parser sees exactly what the author wrote.
    fn string(&mut self) -> Result<TokenKind<'input>, Diagnostic> {
        let start = self.column();
        self.bump();
        let mut literal = String::new();
        let mut parts: Vec<RawPart> = Vec::new();
        while let Some(ch) = self.peek_char() {
            match ch {
                '"' => {
                    self.bump();
                    if parts.is_empty() {
                        return Ok(TokenKind::String(literal));
                    }
                    if !literal.is_empty() {
                        parts.push(RawPart::Literal(std::mem::take(&mut literal)));
                    }
                    return Ok(TokenKind::Interpolated(parts));
                }
                '\\' => {
                    self.bump();
                    literal.push(self.escape(start)?);
                }
                '[' => {
                    self.bump();
                    if self.peek_char() == Some('[') {
                        self.bump();
                        literal.push('[');
                    } else {
                        if !literal.is_empty() {
                            parts.push(RawPart::Literal(std::mem::take(&mut literal)));
                        }
                        parts.push(self.hole()?);
                    }
                }
                ']' if self.peek_ahead(1) == Some(']') => {
                    // Symmetric escape for a literal `]`, so authors can write
                    // `]]` without it ever being read as a stray bracket.
                    self.bump();
                    self.bump();
                    literal.push(']');
                }
                _ => {
                    self.bump();
                    literal.push(ch);
                }
            }
        }
        Err(self.error_at(start, "unterminated string literal"))
    }

    /// Reads one escape body after a `\\` inside a string literal.
    fn escape(&mut self, start: usize) -> Result<char, Diagnostic> {
        let escaped = self
            .bump()
            .ok_or_else(|| self.error_at(start, "unterminated string literal"))?;
        match escaped {
            'n' => Ok('\n'),
            'r' => Ok('\r'),
            't' => Ok('\t'),
            '"' => Ok('"'),
            '\\' => Ok('\\'),
            _ => Err(self.error_at(
                self.column().saturating_sub(1),
                format!("unsupported escape `\\{escaped}`"),
            )),
        }
    }

    /// Reads a `[expr]` hole body (the opening `[` is already consumed),
    /// capturing its raw text and source column. Nested `[` / `]` are balanced
    /// so a hole may contain bracketed sub-expressions.
    fn hole(&mut self) -> Result<RawPart, Diagnostic> {
        let column = self.column();
        let mut text = String::new();
        let mut depth = 1usize;
        let mut in_string = false;
        let mut escaped = false;
        while let Some(ch) = self.bump() {
            if in_string {
                text.push(ch);
                if escaped {
                    escaped = false;
                } else if ch == '\\' {
                    escaped = true;
                } else if ch == '"' {
                    in_string = false;
                }
                continue;
            }
            match ch {
                '"' => {
                    in_string = true;
                    text.push(ch);
                }
                '[' => {
                    depth += 1;
                    text.push(ch);
                }
                ']' => {
                    depth -= 1;
                    if depth == 0 {
                        return Ok(RawPart::Hole { text, column });
                    }
                    text.push(ch);
                }
                _ => text.push(ch),
            }
        }
        Err(self.error_at(column, "unterminated `[...]` interpolation"))
    }

    fn peek_ahead(&self, offset: usize) -> Option<char> {
        self.input[self.offset..].chars().nth(offset)
    }

    fn integer(&mut self) -> Result<i64, Diagnostic> {
        let start = self.offset;
        let column = self.column();
        while self.peek_char().is_some_and(|ch| ch.is_ascii_digit()) {
            self.bump();
        }
        self.input[start..self.offset]
            .parse()
            .map_err(|_| self.error_at(column, "integer is out of range"))
    }

    fn identifier(&mut self) -> &'input str {
        let start = self.offset;
        while self.peek_char().is_some_and(is_identifier_continue) {
            self.bump();
        }
        &self.input[start..self.offset]
    }

    fn peek_char(&self) -> Option<char> {
        self.input[self.offset..].chars().next()
    }

    fn bump(&mut self) -> Option<char> {
        let ch = self.peek_char()?;
        self.offset += ch.len_utf8();
        self.current_column += 1;
        Some(ch)
    }

    const fn column(&self) -> usize {
        self.current_column
    }

    fn error_at(&self, column: usize, message: impl Into<String>) -> Diagnostic {
        Diagnostic::new(self.file, self.line, column, message)
    }
}

const fn is_identifier_start(ch: char) -> bool {
    ch == '_' || ch.is_ascii_alphabetic()
}

const fn is_identifier_continue(ch: char) -> bool {
    is_identifier_start(ch) || ch.is_ascii_digit()
}
