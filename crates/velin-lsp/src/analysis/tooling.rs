//! Protocol-neutral editor analysis shared by LSP request handlers.

use super::SourceRange;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SemanticTokenKind {
    Keyword,
    Variable,
    Function,
    String,
    Number,
    Operator,
    Label,
    Namespace,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SemanticToken {
    pub range: SourceRange,
    pub kind: SemanticTokenKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SymbolKind {
    Label,
    Function,
    Variable,
    Module,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NamedSymbol {
    pub name: String,
    pub range: SourceRange,
    pub kind: SymbolKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Lexeme {
    text: String,
    range: SourceRange,
    raw_end: usize,
    shape: LexemeShape,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LexemeShape {
    Identifier,
    String,
    Number,
    Operator,
}

#[must_use]
pub fn semantic_tokens(text: &str) -> Vec<SemanticToken> {
    let lexemes = lex(text);
    lexemes
        .iter()
        .enumerate()
        .map(|(index, lexeme)| SemanticToken {
            range: lexeme.range,
            kind: classify(&lexemes, index),
        })
        .collect()
}

#[must_use]
pub fn symbols(text: &str) -> Vec<NamedSymbol> {
    let lexemes = lex(text);
    let mut symbols = Vec::new();
    for (index, lexeme) in lexemes.iter().enumerate() {
        if lexeme.shape != LexemeShape::Identifier {
            continue;
        }
        let previous = index
            .checked_sub(1)
            .and_then(|at| lexemes.get(at))
            .filter(|item| item.range.line == lexeme.range.line);
        let kind = match previous.map(|item| item.text.as_str()) {
            Some("label") => Some(SymbolKind::Label),
            Some("fn") => Some(SymbolKind::Function),
            Some("import") => Some(SymbolKind::Module),
            Some("default" | "set" | "for") => Some(SymbolKind::Variable),
            _ if is_assignment_target(text, lexeme) => Some(SymbolKind::Variable),
            _ => None,
        };
        if let Some(kind) = kind {
            symbols.push(NamedSymbol {
                name: lexeme.text.clone(),
                range: lexeme.range,
                kind,
            });
        }
    }
    symbols
}

#[must_use]
pub fn identifier_occurrences(text: &str, name: &str) -> Vec<SourceRange> {
    lex(text)
        .into_iter()
        .filter(|lexeme| lexeme.shape == LexemeShape::Identifier && lexeme.text == name)
        .map(|lexeme| lexeme.range)
        .collect()
}

#[must_use]
pub fn identifier_at(text: &str, line: usize, column: usize) -> Option<(String, SourceRange)> {
    lex(text).into_iter().find_map(|lexeme| {
        (lexeme.shape == LexemeShape::Identifier
            && lexeme.range.line == line
            && column >= lexeme.range.start_column
            && column <= lexeme.range.end_column)
            .then_some((lexeme.text, lexeme.range))
    })
}

#[must_use]
pub fn definition(text: &str, line: usize, column: usize) -> Option<SourceRange> {
    let (name, _) = identifier_at(text, line, column)?;
    symbols(text)
        .into_iter()
        .find(|symbol| symbol.name == name)
        .map(|symbol| symbol.range)
}

#[must_use]
pub fn valid_rename(name: &str) -> bool {
    let mut characters = name.chars();
    characters
        .next()
        .is_some_and(|character| character.is_ascii_alphabetic() || character == '_')
        && characters.all(|character| character.is_ascii_alphanumeric() || character == '_')
        && !super::KEYWORDS.contains(&name)
        && !super::BUILTINS.contains(&name)
        && !matches!(name, "true" | "false" | "and" | "or" | "not" | "in")
}

fn classify(lexemes: &[Lexeme], index: usize) -> SemanticTokenKind {
    let lexeme = &lexemes[index];
    match lexeme.shape {
        LexemeShape::String => SemanticTokenKind::String,
        LexemeShape::Number => SemanticTokenKind::Number,
        LexemeShape::Operator => SemanticTokenKind::Operator,
        LexemeShape::Identifier => {
            if super::KEYWORDS.contains(&lexeme.text.as_str())
                || matches!(
                    lexeme.text.as_str(),
                    "true" | "false" | "and" | "or" | "not" | "in"
                )
            {
                return SemanticTokenKind::Keyword;
            }
            let previous = index
                .checked_sub(1)
                .and_then(|at| lexemes.get(at))
                .filter(|item| item.range.line == lexeme.range.line);
            let next = lexemes
                .get(index + 1)
                .filter(|item| item.range.line == lexeme.range.line);
            if previous.is_some_and(|item| matches!(item.text.as_str(), "label" | "jump")) {
                SemanticTokenKind::Label
            } else if previous.is_some_and(|item| item.text == "import")
                || next.is_some_and(|item| item.text == ".")
            {
                SemanticTokenKind::Namespace
            } else if previous.is_some_and(|item| item.text == "fn")
                || next.is_some_and(|item| item.text == "(")
            {
                SemanticTokenKind::Function
            } else {
                SemanticTokenKind::Variable
            }
        }
    }
}

fn is_assignment_target(text: &str, lexeme: &Lexeme) -> bool {
    let Some(line) = text.lines().nth(lexeme.range.line.saturating_sub(1)) else {
        return false;
    };
    line.get(lexeme.raw_end..)
        .is_some_and(|rest| rest.trim_start().starts_with('='))
}

fn lex(text: &str) -> Vec<Lexeme> {
    let mut output = Vec::new();
    for (line_index, line) in text.lines().enumerate() {
        lex_line(line, line_index + 1, &mut output);
    }
    output
}

fn lex_line(line: &str, line_number: usize, output: &mut Vec<Lexeme>) {
    let characters: Vec<(usize, char)> = line.char_indices().collect();
    let mut at = 0;
    while at < characters.len() {
        let (start_byte, character) = characters[at];
        if character.is_whitespace() {
            at += 1;
            continue;
        }
        if character == '#' {
            break;
        }
        let start_column = at + 1;
        if character == '"' {
            at = string_end(&characters, at + 1);
            push_lexeme(
                line,
                line_number,
                start_column,
                start_byte,
                at,
                &characters,
                LexemeShape::String,
                output,
            );
            continue;
        }
        if character.is_ascii_alphabetic() || character == '_' {
            at += 1;
            while at < characters.len()
                && (characters[at].1.is_ascii_alphanumeric() || characters[at].1 == '_')
            {
                at += 1;
            }
            push_lexeme(
                line,
                line_number,
                start_column,
                start_byte,
                at,
                &characters,
                LexemeShape::Identifier,
                output,
            );
            continue;
        }
        if character.is_ascii_digit() {
            at += 1;
            while at < characters.len() && characters[at].1.is_ascii_digit() {
                at += 1;
            }
            push_lexeme(
                line,
                line_number,
                start_column,
                start_byte,
                at,
                &characters,
                LexemeShape::Number,
                output,
            );
            continue;
        }
        at += 1;
        if at < characters.len()
            && matches!(
                (character, characters[at].1),
                ('=' | '!' | '<' | '>' | '+' | '-' | '*' | '/', '=')
            )
        {
            at += 1;
        }
        push_lexeme(
            line,
            line_number,
            start_column,
            start_byte,
            at,
            &characters,
            LexemeShape::Operator,
            output,
        );
    }
}

fn string_end(characters: &[(usize, char)], mut at: usize) -> usize {
    let mut escaped = false;
    while at < characters.len() {
        let current = characters[at].1;
        at += 1;
        if escaped {
            escaped = false;
        } else if current == '\\' {
            escaped = true;
        } else if current == '"' {
            break;
        }
    }
    at
}

#[allow(clippy::too_many_arguments)]
fn push_lexeme(
    line: &str,
    line_number: usize,
    start_column: usize,
    start_byte: usize,
    at: usize,
    characters: &[(usize, char)],
    shape: LexemeShape,
    output: &mut Vec<Lexeme>,
) {
    let end_byte = characters.get(at).map_or(line.len(), |item| item.0);
    output.push(Lexeme {
        text: line[start_byte..end_byte].to_owned(),
        range: SourceRange {
            line: line_number,
            start_column,
            end_column: at + 1,
        },
        raw_end: end_byte,
        shape,
    });
}
