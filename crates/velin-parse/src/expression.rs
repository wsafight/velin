use velin_syntax::Builtin;
use velin_syntax::diagnostic::Diagnostic;
use velin_syntax::expr::{BinaryOp, Expr, Span, StrPart, UnaryOp, Value};

#[derive(Debug, Clone, PartialEq, Eq)]
enum TokenKind {
    Integer(i64),
    Boolean(bool),
    String(String),
    /// A string literal containing at least one `[expr]` hole. Literal segments
    /// are captured verbatim; each hole keeps its raw source text and starting
    /// column so it can be parsed as a sub-expression with correct diagnostics.
    Interpolated(Vec<RawPart>),
    Identifier(String),
    LeftParen,
    RightParen,
    Comma,
    Plus,
    Minus,
    Star,
    Slash,
    Equal,
    NotEqual,
    Less,
    LessEqual,
    Greater,
    GreaterEqual,
    And,
    Or,
    Not,
    End,
}

/// A raw piece of an interpolated string, before holes are parsed.
#[derive(Debug, Clone, PartialEq, Eq)]
enum RawPart {
    Literal(String),
    Hole { text: String, column: usize },
}

#[derive(Debug, Clone)]
struct Token {
    kind: TokenKind,
    column: usize,
}

/// Maximum UTF-8 size of one expression, including interpolation source.
pub const MAX_EXPRESSION_BYTES: usize = 65_536;
/// Maximum tokens in one expression parser invocation.
pub const MAX_EXPRESSION_TOKENS: usize = 512;
/// Maximum parenthesis nesting in one expression parser invocation.
pub const MAX_EXPRESSION_NESTING: usize = 32;
/// Maximum recursively nested string interpolation expressions.
pub const MAX_INTERPOLATION_DEPTH: usize = 32;
/// Maximum cumulative input bytes reprocessed by nested interpolation parsers.
pub const MAX_EXPRESSION_WORK_BYTES: usize = MAX_EXPRESSION_BYTES * 4;
/// Maximum cumulative tokens produced across nested interpolation parsers.
pub const MAX_EXPRESSION_TOTAL_TOKENS: usize = MAX_EXPRESSION_TOKENS * 4;

struct ParseBudget {
    work_bytes: usize,
    tokens: usize,
}

/// Parses a bounded expression at its source location.
/// # Errors
/// Returns a diagnostic for invalid syntax.
pub fn parse_expression(
    input: &str,
    file: &str,
    line: usize,
    base_column: usize,
) -> Result<Expr, Diagnostic> {
    parse_expression_inner(
        input,
        file,
        line,
        base_column,
        0,
        &mut ParseBudget {
            work_bytes: 0,
            tokens: 0,
        },
    )
}

fn parse_expression_inner(
    input: &str,
    file: &str,
    line: usize,
    base_column: usize,
    interpolation_depth: usize,
    budget: &mut ParseBudget,
) -> Result<Expr, Diagnostic> {
    if input.len() > MAX_EXPRESSION_BYTES {
        return Err(Diagnostic::new(
            file,
            line,
            base_column,
            "expression exceeds 64 KiB",
        ));
    }
    if interpolation_depth > MAX_INTERPOLATION_DEPTH {
        return Err(Diagnostic::new(
            file,
            line,
            base_column,
            format!("interpolation nesting exceeds {MAX_INTERPOLATION_DEPTH}"),
        ));
    }
    budget.work_bytes = budget
        .work_bytes
        .checked_add(input.len())
        .filter(|bytes| *bytes <= MAX_EXPRESSION_WORK_BYTES)
        .ok_or_else(|| {
            Diagnostic::new(
                file,
                line,
                base_column,
                "nested interpolation exceeds expression work budget",
            )
        })?;
    let tokens = Lexer::new(input, file, line, base_column).lex()?;
    if tokens.len() > MAX_EXPRESSION_TOKENS {
        return Err(Diagnostic::new(
            file,
            line,
            base_column,
            "expression exceeds 512 tokens",
        ));
    }
    budget.tokens = budget
        .tokens
        .checked_add(tokens.len())
        .filter(|tokens| *tokens <= MAX_EXPRESSION_TOTAL_TOKENS)
        .ok_or_else(|| {
            Diagnostic::new(
                file,
                line,
                base_column,
                "nested interpolation exceeds total token budget",
            )
        })?;
    let mut depth: usize = 0;
    for token in &tokens {
        if token.kind == TokenKind::LeftParen {
            depth += 1;
        }
        if depth > MAX_EXPRESSION_NESTING {
            return Err(Diagnostic::new(
                file,
                line,
                token.column,
                "expression nesting exceeds 32",
            ));
        }
        if token.kind == TokenKind::RightParen {
            depth = depth.saturating_sub(1);
        }
    }
    let mut parser = ExpressionParser {
        tokens,
        current: 0,
        file,
        line,
        interpolation_depth,
        budget,
    };
    let expression = parser.parse_or()?;
    if parser.peek().kind != TokenKind::End {
        return Err(parser.error("unexpected token after expression"));
    }
    Ok(expression)
}

struct Lexer<'a> {
    input: &'a str,
    file: &'a str,
    line: usize,
    base_column: usize,
    offset: usize,
}

impl<'a> Lexer<'a> {
    const fn new(input: &'a str, file: &'a str, line: usize, base_column: usize) -> Self {
        Self {
            input,
            file,
            line,
            base_column,
            offset: 0,
        }
    }

    fn lex(mut self) -> Result<Vec<Token>, Diagnostic> {
        let mut tokens = Vec::new();
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
                    match word.as_str() {
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
    fn string(&mut self) -> Result<TokenKind, Diagnostic> {
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
        while self.peek_char().is_some_and(|ch| ch.is_ascii_digit()) {
            self.bump();
        }
        self.input[start..self.offset]
            .parse()
            .map_err(|_| self.error_at(self.column_at(start), "integer is out of range"))
    }

    fn identifier(&mut self) -> String {
        let start = self.offset;
        while self.peek_char().is_some_and(is_identifier_continue) {
            self.bump();
        }
        self.input[start..self.offset].to_owned()
    }

    fn peek_char(&self) -> Option<char> {
        self.input[self.offset..].chars().next()
    }

    fn bump(&mut self) -> Option<char> {
        let ch = self.peek_char()?;
        self.offset += ch.len_utf8();
        Some(ch)
    }

    fn column(&self) -> usize {
        self.column_at(self.offset)
    }

    fn column_at(&self, byte_offset: usize) -> usize {
        self.base_column + self.input[..byte_offset].chars().count()
    }

    fn error_at(&self, column: usize, message: impl Into<String>) -> Diagnostic {
        Diagnostic::new(self.file, self.line, column, message)
    }
}

struct ExpressionParser<'a, 'budget> {
    tokens: Vec<Token>,
    current: usize,
    file: &'a str,
    line: usize,
    interpolation_depth: usize,
    budget: &'budget mut ParseBudget,
}

impl ExpressionParser<'_, '_> {
    fn parse_or(&mut self) -> Result<Expr, Diagnostic> {
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
        self.parse_primary()
    }

    fn parse_primary(&mut self) -> Result<Expr, Diagnostic> {
        let token = self.advance().clone();
        let span = self.span(token.column);
        match token.kind {
            TokenKind::Integer(value) => Ok(Expr::Value(Value::Integer(value)).spanned(span)),
            TokenKind::Boolean(value) => Ok(Expr::Value(Value::Boolean(value)).spanned(span)),
            TokenKind::String(value) => Ok(Expr::Value(Value::String(value)).spanned(span)),
            TokenKind::Interpolated(raw) => Ok(self.interpolate(raw)?.spanned(span)),
            TokenKind::Identifier(value) if self.consume(&TokenKind::LeftParen) => {
                let function = Builtin::named(&value)
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
            TokenKind::Identifier(value) => Ok(Expr::Variable(value).spanned(span)),
            TokenKind::LeftParen => {
                let expression = self.parse_or()?;
                if !self.consume(&TokenKind::RightParen) {
                    return Err(self.error("expected `)`"));
                }
                Ok(expression)
            }
            _ => Err(Diagnostic::new(
                self.file,
                self.line,
                token.column,
                "expected a value, variable, or parenthesized expression",
            )),
        }
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

    fn consume(&mut self, kind: &TokenKind) -> bool {
        if &self.peek().kind == kind {
            self.current += 1;
            true
        } else {
            false
        }
    }

    fn consume_column(&mut self, kind: &TokenKind) -> Option<usize> {
        if &self.peek().kind != kind {
            return None;
        }
        let column = self.peek().column;
        self.current += 1;
        Some(column)
    }

    fn advance(&mut self) -> &Token {
        let index = self.current;
        if self.tokens[index].kind != TokenKind::End {
            self.current += 1;
        }
        &self.tokens[index]
    }

    fn peek(&self) -> &Token {
        &self.tokens[self.current]
    }

    fn error(&self, message: impl Into<String>) -> Diagnostic {
        Diagnostic::new(self.file, self.line, self.peek().column, message)
    }

    fn span(&self, column: usize) -> Span {
        Span::in_source(self.file, self.line, column)
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

const fn is_identifier_start(ch: char) -> bool {
    ch == '_' || ch.is_ascii_alphabetic()
}

const fn is_identifier_continue(ch: char) -> bool {
    is_identifier_start(ch) || ch.is_ascii_digit()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(input: &str) -> Result<Expr, Diagnostic> {
        parse_expression(input, "test.rl", 1, 1)
    }

    #[test]
    fn parses_arithmetic_with_precedence() {
        let expr = parse("1 + 2 * 3").unwrap();
        // 1 + (2 * 3)
        let Expr::Binary { op, right, .. } = expr.unspanned() else {
            panic!("expected binary");
        };
        assert_eq!(*op, BinaryOp::Add);
        assert!(matches!(
            right.unspanned(),
            Expr::Binary {
                op: BinaryOp::Multiply,
                ..
            }
        ));
    }

    #[test]
    fn parses_builtin_calls_and_rejects_unknown() {
        assert!(parse("len(bag)").is_ok());
        assert!(parse("push(bag, \"x\")").is_ok());
        assert!(parse("random(1, 6)").is_ok());
        assert!(parse("chance(50)").is_ok());
        assert!(parse("random(1)").is_err());
        assert!(parse("chance(1, 2)").is_err());
        assert!(parse("exec(\"cmd\")").is_err());
    }

    #[test]
    fn parses_boolean_and_comparison_operators() {
        assert!(parse("score >= 1 and not done").is_ok());
        assert!(parse("a == b or c != d").is_ok());
    }

    #[test]
    fn rejects_unbalanced_and_trailing_tokens() {
        assert!(parse("(1 + 2").is_err());
        assert!(parse("1 2").is_err());
        assert!(parse("=").is_err());
    }

    #[test]
    fn plain_string_without_holes_stays_a_value() {
        // Zero-regression guarantee: no `[expr]` means a plain string value.
        assert_eq!(
            parse("\"just text\"").unwrap().into_unspanned(),
            Expr::Value(Value::String("just text".into()))
        );
        // `[[` / `]]` escape to literal brackets and keep it a plain value.
        assert_eq!(
            parse("\"a [[b]] c\"").unwrap().into_unspanned(),
            Expr::Value(Value::String("a [b] c".into()))
        );
    }

    #[test]
    fn interpolation_splits_literals_and_holes() {
        let expression = parse("\"hp is [hp] now\"").unwrap();
        let Expr::Interpolate { parts } = expression.unspanned() else {
            panic!("expected interpolation");
        };
        assert_eq!(parts.len(), 3);
        assert_eq!(parts[0], StrPart::Literal("hp is ".into()));
        assert_eq!(parts[2], StrPart::Literal(" now".into()));
        match &parts[1] {
            StrPart::Hole(expr) => {
                assert_eq!(expr.unspanned(), &Expr::Variable("hp".into()));
            }
            StrPart::Literal(text) => panic!("expected hole, got literal {text:?}"),
        }
    }

    #[test]
    fn interpolation_holes_carry_full_expressions_and_report_errors() {
        // A hole is a full sub-expression, so operators and builtins work.
        let expression = parse("\"[hp + 1]\"").unwrap();
        let Expr::Interpolate { parts } = expression.unspanned() else {
            panic!("expected interpolation");
        };
        assert!(matches!(
            &parts[0],
            StrPart::Hole(expr) if matches!(expr.unspanned(), Expr::Binary { op: BinaryOp::Add, .. })
        ));
        // An unterminated hole and a broken sub-expression are both errors.
        assert!(parse("\"open [hp\"").is_err());
        assert!(parse("\"[1 +]\"").is_err());
    }

    #[test]
    fn brackets_inside_hole_strings_do_not_close_the_hole() {
        let expression = parse(r#""[contains("a]", "x")]""#).unwrap();
        let Expr::Interpolate { parts } = expression.unspanned() else {
            panic!("expected interpolation");
        };
        assert_eq!(parts.len(), 1);

        let expression = parse(r#""[contains("a\"b]", "x")]""#).unwrap();
        let Expr::Interpolate { parts } = expression.unspanned() else {
            panic!("expected interpolation");
        };
        assert_eq!(parts.len(), 1);
    }

    #[test]
    fn diagnostics_use_unicode_character_columns() {
        let ascii = parse_expression("\"a\" +", "test.rl", 1, 9).unwrap_err();
        let unicode = parse_expression("\"\u{4f60}\" +", "test.rl", 1, 9).unwrap_err();

        assert_eq!(ascii.column, 14);
        assert_eq!(unicode.column, 14);
    }

    #[test]
    fn lexer_and_parser_reject_budget_and_token_errors() {
        assert!(
            parse(&"x".repeat(65_537))
                .unwrap_err()
                .message
                .contains("64 KiB")
        );
        let too_many_tokens = (0..300).map(|_| "1+").collect::<String>() + "1";
        assert!(
            parse(&too_many_tokens)
                .unwrap_err()
                .message
                .contains("512 tokens")
        );
        let nested = format!("{}1{}", "(".repeat(33), ")".repeat(33));
        assert!(parse(&nested).unwrap_err().message.contains("nesting"));
        assert!(
            parse("@")
                .unwrap_err()
                .message
                .contains("unexpected character")
        );
        assert!(parse("1 = 2").unwrap_err().message.contains("`==`"));
        assert!(parse("1 ! 2").unwrap_err().message.contains("`!=`"));
        assert!(
            parse("\"abc")
                .unwrap_err()
                .message
                .contains("unterminated string")
        );
        assert!(
            parse("\"\\q\"")
                .unwrap_err()
                .message
                .contains("unsupported escape")
        );
        assert!(
            parse("9223372036854775808")
                .unwrap_err()
                .message
                .contains("out of range")
        );
        assert!(
            parse("len(1 2)")
                .unwrap_err()
                .message
                .contains("expected `,` or `)`")
        );
    }

    #[test]
    fn nested_interpolation_uses_one_shared_budget() {
        let mut expression = "1".to_owned();
        for _ in 0..=MAX_INTERPOLATION_DEPTH {
            expression = format!(r#""[{expression}]""#);
        }
        let error = parse(&expression).unwrap_err();
        assert!(error.message.contains("interpolation nesting"));
    }

    #[test]
    fn string_escapes_and_nested_holes_parse() {
        assert_eq!(
            parse(r#""a\n\r\t\"\\b""#).unwrap().into_unspanned(),
            Expr::Value(Value::String("a\n\r\t\"\\b".into()))
        );
        let expression = parse(r#""pre[hp]post""#).unwrap();
        let Expr::Interpolate { parts } = expression.unspanned() else {
            panic!("expected interpolation");
        };
        assert_eq!(parts[0], StrPart::Literal("pre".into()));
        assert_eq!(parts[2], StrPart::Literal("post".into()));
        let expression = parse(r#""[list(1, list(2))]""#).unwrap();
        let Expr::Interpolate { parts } = expression.unspanned() else {
            panic!("expected nested hole");
        };
        assert_eq!(parts.len(), 1);
        assert!(parse("\"[hp\"").is_err());
    }
}
