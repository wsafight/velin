use velin_syntax::Builtin;
use velin_syntax::diagnostic::Diagnostic;
use velin_syntax::expr::{BinaryOp, Expr, SharedString, Span, StrPart, UnaryOp, Value};

#[derive(Debug, Clone, PartialEq, Eq)]
enum TokenKind<'input> {
    Integer(i64),
    Boolean(bool),
    String(String),
    /// A string literal containing at least one `[expr]` hole. Literal segments
    /// are captured verbatim; each hole keeps its raw source text and starting
    /// column so it can be parsed as a sub-expression with correct diagnostics.
    Interpolated(Vec<RawPart>),
    Identifier(&'input str),
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
struct Token<'input> {
    kind: TokenKind<'input>,
    column: usize,
}

const INLINE_TOKENS: usize = 4;

/// Keeps the common short expression entirely on the stack. Once full, all
/// tokens move to a `Vec` and parsing continues through the same interface.
struct TokenBuffer<'input> {
    inline: [Token<'input>; INLINE_TOKENS],
    len: usize,
    spill: Option<Vec<Token<'input>>>,
    spill_capacity: usize,
}

impl<'input> TokenBuffer<'input> {
    fn new(input_bytes: usize) -> Self {
        Self {
            inline: std::array::from_fn(|_| Token {
                kind: TokenKind::End,
                column: 1,
            }),
            len: 0,
            spill: None,
            spill_capacity: input_bytes.div_ceil(2).saturating_add(1),
        }
    }

    fn push(&mut self, token: Token<'input>) {
        if let Some(tokens) = &mut self.spill {
            tokens.push(token);
        } else if self.len < INLINE_TOKENS {
            self.inline[self.len] = token;
        } else {
            let mut tokens = Vec::with_capacity(self.spill_capacity.max(INLINE_TOKENS + 1));
            for inline in &mut self.inline {
                tokens.push(Token {
                    kind: std::mem::replace(&mut inline.kind, TokenKind::End),
                    column: inline.column,
                });
            }
            tokens.push(token);
            self.spill = Some(tokens);
        }
        self.len += 1;
    }

    const fn len(&self) -> usize {
        self.len
    }

    fn get(&self, index: usize) -> &Token<'input> {
        match &self.spill {
            Some(tokens) => &tokens[index],
            None => &self.inline[index],
        }
    }

    fn take(&mut self, index: usize) -> Token<'input> {
        let token = match &mut self.spill {
            Some(tokens) => &mut tokens[index],
            None => &mut self.inline[index],
        };
        Token {
            kind: std::mem::replace(&mut token.kind, TokenKind::End),
            column: token.column,
        }
    }
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
    let source = SharedString::from(file);
    parse_expression_with_source(input, &source, line, base_column)
}

/// Parses an expression while reusing a source name shared by a larger
/// statement parser.
///
/// # Errors
/// Returns the same diagnostics as [`parse_expression`].
pub fn parse_expression_with_source(
    input: &str,
    source: &SharedString,
    line: usize,
    base_column: usize,
) -> Result<Expr, Diagnostic> {
    parse_expression_inner(
        input,
        source.as_str(),
        source,
        line,
        base_column,
        0,
        &mut ParseBudget {
            work_bytes: 0,
            tokens: 0,
        },
    )
}

/// Parses a comma-separated expression list while sharing a source name.
///
/// This is the argument grammar used by host calls. It accepts the same
/// maximum of 128 expressions as `list(...)` without constructing a synthetic
/// wrapper expression.
///
/// # Errors
/// Returns a diagnostic for invalid syntax or an exceeded expression budget.
pub fn parse_expression_list_with_source(
    input: &str,
    source: &SharedString,
    line: usize,
    base_column: usize,
) -> Result<Vec<Expr>, Diagnostic> {
    let mut budget = ParseBudget {
        work_bytes: 0,
        tokens: 0,
    };
    let mut parser = prepare_parser(
        input,
        source.as_str(),
        source,
        line,
        base_column,
        0,
        &mut budget,
    )?;
    let mut expressions = Vec::new();
    if parser.peek().kind != TokenKind::End {
        loop {
            expressions.push(parser.parse_or()?);
            if parser.peek().kind == TokenKind::End {
                break;
            }
            if !parser.consume(&TokenKind::Comma) {
                return Err(parser.error("expected `,` or `)`"));
            }
        }
    }
    if !Builtin::List.accepts(expressions.len()) {
        return Err(parser.error("invalid argument count for `list`"));
    }
    Ok(expressions)
}

fn parse_expression_inner(
    input: &str,
    file: &str,
    source: &SharedString,
    line: usize,
    base_column: usize,
    interpolation_depth: usize,
    budget: &mut ParseBudget,
) -> Result<Expr, Diagnostic> {
    let mut parser = prepare_parser(
        input,
        file,
        source,
        line,
        base_column,
        interpolation_depth,
        budget,
    )?;
    let expression = parser.parse_or()?;
    if parser.peek().kind != TokenKind::End {
        return Err(parser.error("unexpected token after expression"));
    }
    Ok(expression)
}

#[allow(clippy::too_many_arguments)]
fn prepare_parser<'input, 'file, 'budget>(
    input: &'input str,
    file: &'file str,
    source: &SharedString,
    line: usize,
    base_column: usize,
    interpolation_depth: usize,
    budget: &'budget mut ParseBudget,
) -> Result<ExpressionParser<'input, 'file, 'budget>, Diagnostic> {
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
    Ok(ExpressionParser {
        tokens,
        current: 0,
        file,
        source: source.clone(),
        line,
        interpolation_depth,
        budget,
    })
}

mod lexer;
mod parser;

use lexer::Lexer;
use parser::ExpressionParser;

#[cfg(test)]
#[path = "expression_tests.rs"]
mod tests;
