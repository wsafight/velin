//! Language analysis: turning a document's text into the three capabilities
//! this server exposes — diagnostics, completions, and document symbols.
//!
//! Every capability is a thin adaptor over the `velin` library: diagnostics are
//! `compile` + `check_script`, completions merge the fixed keyword/builtin
//! vocabulary with the identifiers a script actually declares, and symbols are
//! the script's labels. The server layer (`server.rs`) only has to translate
//! these into LSP JSON — no language logic lives there.

use velin::{Diagnostic, Stmt, check_script, compile, parse_program_recovering};

/// The statement keywords the surface language recognises, offered as
/// completions regardless of parse state.
pub const KEYWORDS: &[&str] = &[
    "label", "default", "set", "perform", "if", "elif", "else", "while", "jump",
];

/// Every built-in function name, offered as a completion.
pub const BUILTINS: &[&str] = &[
    "list", "record", "get", "put", "push", "remove", "len", "contains", "random", "chance",
];

/// What a completion item stands for, mapped to an LSP `CompletionItemKind` by
/// the server. Kept as a small enum so `analysis` stays protocol-agnostic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompletionKind {
    Keyword,
    Function,
    Variable,
    Label,
}

/// One completion suggestion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Completion {
    pub label: String,
    pub kind: CompletionKind,
    pub detail: &'static str,
}

/// A named jump target with the line it is defined on (1-based), for the
/// document-symbol outline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LabelSymbol {
    pub name: String,
    pub line: usize,
}

/// A source range using Velin's 1-based Unicode-scalar coordinates. The end
/// column is exclusive, matching LSP range semantics after conversion.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SourceRange {
    pub line: usize,
    pub start_column: usize,
    pub end_column: usize,
}

/// Markdown hover text and the identifier range it describes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hover {
    pub contents: String,
    pub range: SourceRange,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct LabelOccurrence {
    name: String,
    range: SourceRange,
    declaration: bool,
}

/// Computes syntax, lowering, and static-check diagnostics for `text`.
/// Recoverable syntax errors are all returned in one pass; lowering and static
/// checks only run once the document is syntactically complete.
#[must_use]
pub fn diagnostics(file: &str, text: &str) -> Vec<Diagnostic> {
    let recovered = parse_program_recovering(text);
    if !recovered.errors.is_empty() {
        return recovered
            .errors
            .into_iter()
            .map(|error| error.into_diagnostic(file))
            .collect();
    }
    match compile(file, text) {
        Ok(script) => check_script(file, &script),
        Err(diagnostic) => vec![diagnostic],
    }
}

/// Builds the completion list for `text`: the fixed keyword and builtin
/// vocabulary plus every variable and label recoverable from the current text.
#[must_use]
pub fn completions(text: &str) -> Vec<Completion> {
    let mut items: Vec<Completion> = KEYWORDS
        .iter()
        .map(|keyword| Completion {
            label: (*keyword).to_owned(),
            kind: CompletionKind::Keyword,
            detail: "keyword",
        })
        .chain(BUILTINS.iter().map(|name| Completion {
            label: (*name).to_owned(),
            kind: CompletionKind::Function,
            detail: "builtin",
        }))
        .collect();

    let recovered = parse_program_recovering(text);
    let mut variables = Vec::new();
    let mut labels = Vec::new();
    collect_names(&recovered.statements, &mut variables, &mut labels);
    variables.sort();
    variables.dedup();
    labels.sort();
    labels.dedup();
    items.extend(variables.into_iter().map(|name| Completion {
        label: name,
        kind: CompletionKind::Variable,
        detail: "variable",
    }));
    items.extend(labels.into_iter().map(|name| Completion {
        label: name,
        kind: CompletionKind::Label,
        detail: "label",
    }));

    items
}

/// Extracts every label recoverable from `text` for the document outline.
#[must_use]
pub fn document_symbols(text: &str) -> Vec<LabelSymbol> {
    let recovered = parse_program_recovering(text);
    let mut symbols = Vec::new();
    collect_labels(&recovered.statements, &mut symbols);
    symbols
}

/// Returns concise language information for the identifier at `line`/`column`.
#[must_use]
pub fn hover(text: &str, line: usize, column: usize) -> Option<Hover> {
    let (word, range) = word_at(text, line, column)?;
    let contents = if let Some(description) = keyword_description(word) {
        format!("`{word}` keyword\n\n{description}")
    } else if let Some(signature) = builtin_signature(word) {
        format!("```velin\n{signature}\n```\n\nDeterministic built-in function.")
    } else {
        let recovered = parse_program_recovering(text);
        let lines: Vec<&str> = text.lines().collect();
        let labels = label_occurrences(&recovered.statements, &lines);
        if let Some(definition) = labels
            .iter()
            .find(|item| item.name == word && item.declaration)
        {
            format!(
                "`{word}` label\n\nJump target defined on line {}.",
                definition.range.line
            )
        } else {
            let mut variables = Vec::new();
            let mut label_names = Vec::new();
            collect_names(&recovered.statements, &mut variables, &mut label_names);
            if variables.iter().any(|name| name == word) {
                format!("`{word}` variable\n\nScript state value.")
            } else {
                let mut hosts = Vec::new();
                collect_hosts(&recovered.statements, &mut hosts);
                if hosts.iter().any(|name| name == word) {
                    format!(
                        "`{word}` host command\n\nBehavior and return type are supplied by the embedder."
                    )
                } else {
                    return None;
                }
            }
        }
    };
    Some(Hover { contents, range })
}

/// Finds the declaration targeted by the label occurrence under the cursor.
#[must_use]
pub fn label_definition(text: &str, line: usize, column: usize) -> Option<SourceRange> {
    let lines: Vec<&str> = text.lines().collect();
    let recovered = parse_program_recovering(text);
    let occurrences = label_occurrences(&recovered.statements, &lines);
    let selected = occurrence_at(&occurrences, line, column)?;
    occurrences
        .iter()
        .find(|item| item.declaration && item.name == selected.name)
        .map(|item| item.range)
}

/// Finds every use of the label occurrence under the cursor.
#[must_use]
pub fn label_references(
    text: &str,
    line: usize,
    column: usize,
    include_declaration: bool,
) -> Vec<SourceRange> {
    let lines: Vec<&str> = text.lines().collect();
    let recovered = parse_program_recovering(text);
    let occurrences = label_occurrences(&recovered.statements, &lines);
    let Some(selected) = occurrence_at(&occurrences, line, column) else {
        return Vec::new();
    };
    occurrences
        .iter()
        .filter(|item| item.name == selected.name && (include_declaration || !item.declaration))
        .map(|item| item.range)
        .collect()
}

/// Walks the statement tree, gathering declared variable and label names.
fn collect_names(statements: &[Stmt], variables: &mut Vec<String>, labels: &mut Vec<String>) {
    for statement in statements {
        match statement {
            Stmt::Label { name, body, .. } => {
                labels.push(name.clone());
                collect_names(body, variables, labels);
            }
            Stmt::Default { name, .. } | Stmt::Set { name, .. } => variables.push(name.clone()),
            Stmt::Perform { bind, .. } => {
                if let Some(name) = bind {
                    variables.push(name.clone());
                }
            }
            Stmt::If {
                branches,
                otherwise,
            } => {
                for branch in branches {
                    collect_names(&branch.body, variables, labels);
                }
                if let Some(body) = otherwise {
                    collect_names(body, variables, labels);
                }
            }
            Stmt::While { body, .. } => collect_names(body, variables, labels),
            Stmt::Jump { .. } => {}
        }
    }
}

/// Walks the statement tree, gathering labels with their definition lines.
fn collect_labels(statements: &[Stmt], symbols: &mut Vec<LabelSymbol>) {
    for statement in statements {
        match statement {
            Stmt::Label { name, body, line } => {
                symbols.push(LabelSymbol {
                    name: name.clone(),
                    line: *line,
                });
                collect_labels(body, symbols);
            }
            Stmt::If {
                branches,
                otherwise,
            } => {
                for branch in branches {
                    collect_labels(&branch.body, symbols);
                }
                if let Some(body) = otherwise {
                    collect_labels(body, symbols);
                }
            }
            Stmt::While { body, .. } => collect_labels(body, symbols),
            _ => {}
        }
    }
}

fn collect_hosts(statements: &[Stmt], hosts: &mut Vec<String>) {
    for statement in statements {
        match statement {
            Stmt::Label { body, .. } | Stmt::While { body, .. } => collect_hosts(body, hosts),
            Stmt::Perform { command, .. } => hosts.push(command.clone()),
            Stmt::If {
                branches,
                otherwise,
            } => {
                for branch in branches {
                    collect_hosts(&branch.body, hosts);
                }
                if let Some(body) = otherwise {
                    collect_hosts(body, hosts);
                }
            }
            Stmt::Default { .. } | Stmt::Set { .. } | Stmt::Jump { .. } => {}
        }
    }
}

fn label_occurrences(statements: &[Stmt], lines: &[&str]) -> Vec<LabelOccurrence> {
    let mut occurrences = Vec::new();
    collect_label_occurrences(statements, lines, &mut occurrences);
    occurrences
}

fn collect_label_occurrences(
    statements: &[Stmt],
    lines: &[&str],
    occurrences: &mut Vec<LabelOccurrence>,
) {
    for statement in statements {
        match statement {
            Stmt::Label { name, body, line } => {
                if let Some(range) = statement_name_range(lines, *line, "label", name) {
                    occurrences.push(LabelOccurrence {
                        name: name.clone(),
                        range,
                        declaration: true,
                    });
                }
                collect_label_occurrences(body, lines, occurrences);
            }
            Stmt::Jump { label, line } => {
                if let Some(range) = statement_name_range(lines, *line, "jump", label) {
                    occurrences.push(LabelOccurrence {
                        name: label.clone(),
                        range,
                        declaration: false,
                    });
                }
            }
            Stmt::If {
                branches,
                otherwise,
            } => {
                for branch in branches {
                    collect_label_occurrences(&branch.body, lines, occurrences);
                }
                if let Some(body) = otherwise {
                    collect_label_occurrences(body, lines, occurrences);
                }
            }
            Stmt::While { body, .. } => collect_label_occurrences(body, lines, occurrences),
            Stmt::Default { .. } | Stmt::Set { .. } | Stmt::Perform { .. } => {}
        }
    }
}

fn statement_name_range(
    lines: &[&str],
    line: usize,
    keyword: &str,
    name: &str,
) -> Option<SourceRange> {
    let raw = lines.get(line.checked_sub(1)?)?;
    let trimmed = raw.trim_start_matches(' ');
    let indentation = raw[..raw.len() - trimmed.len()].chars().count();
    let rest = trimmed.strip_prefix(keyword)?;
    let spacing_bytes = rest.len() - rest.trim_start().len();
    let spacing = rest[..spacing_bytes].chars().count();
    let candidate = rest.trim_start();
    if !candidate.starts_with(name)
        || candidate
            .chars()
            .nth(name.chars().count())
            .is_some_and(is_identifier_char)
    {
        return None;
    }
    let start_column = indentation + keyword.chars().count() + spacing + 1;
    Some(SourceRange {
        line,
        start_column,
        end_column: start_column + name.chars().count(),
    })
}

fn occurrence_at(
    occurrences: &[LabelOccurrence],
    line: usize,
    column: usize,
) -> Option<&LabelOccurrence> {
    occurrences.iter().find(|item| {
        item.range.line == line
            && column >= item.range.start_column
            && column <= item.range.end_column
    })
}

fn word_at(text: &str, line: usize, column: usize) -> Option<(&str, SourceRange)> {
    let source_line = text.lines().nth(line.checked_sub(1)?)?;
    let mut cursor = column.saturating_sub(1);
    let char_count = source_line.chars().count();
    if cursor > char_count {
        return None;
    }
    if cursor == char_count
        || !source_line
            .chars()
            .nth(cursor)
            .is_some_and(is_identifier_char)
    {
        cursor = cursor.checked_sub(1)?;
    }
    if !source_line
        .chars()
        .nth(cursor)
        .is_some_and(is_identifier_char)
    {
        return None;
    }

    let chars: Vec<(usize, char)> = source_line.char_indices().collect();
    let mut start = cursor;
    while start > 0 && is_identifier_char(chars[start - 1].1) {
        start -= 1;
    }
    let mut end = cursor + 1;
    while end < chars.len() && is_identifier_char(chars[end].1) {
        end += 1;
    }
    let start_byte = chars[start].0;
    let end_byte = chars.get(end).map_or(source_line.len(), |item| item.0);
    Some((
        &source_line[start_byte..end_byte],
        SourceRange {
            line,
            start_column: start + 1,
            end_column: end + 1,
        },
    ))
}

const fn is_identifier_char(character: char) -> bool {
    character.is_ascii_alphanumeric() || character == '_'
}

fn keyword_description(keyword: &str) -> Option<&'static str> {
    Some(match keyword {
        "label" => "Declares a named target for `jump`.",
        "default" => "Declares a top-level initial value when the host did not provide one.",
        "set" => "Assigns an expression result to script state.",
        "perform" => "Yields a command to the embedding host.",
        "if" => "Runs the first branch whose condition is true.",
        "elif" => "Adds a conditional branch to an `if` statement.",
        "else" => "Adds the fallback branch of an `if` statement.",
        "while" => "Repeats its block while the condition is true.",
        "jump" => "Transfers execution to a named label.",
        _ => return None,
    })
}

fn builtin_signature(name: &str) -> Option<&'static str> {
    Some(match name {
        "list" => "list(values...)",
        "record" => "record(key, value, ...)",
        "get" => "get(collection, key[, fallback])",
        "put" => "put(collection, key, value)",
        "push" => "push(list, value)",
        "remove" => "remove(collection, key)",
        "len" => "len(collection)",
        "contains" => "contains(collection, value)",
        "random" => "random(min, max)",
        "chance" => "chance(percent)",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCRIPT: &str = "\
default hp = 30
label start:
    choice = perform ask(\"go?\")
    if choice == 1:
        set hp = hp + 10
    while hp > 0:
        set hp = hp - 1
label done:
    perform say(\"bye\")
";

    #[test]
    fn a_clean_script_has_no_error_diagnostics() {
        let diagnostics = diagnostics("t.velin", SCRIPT);
        assert!(diagnostics.iter().all(|d| !d.is_error()), "{diagnostics:?}");
    }

    #[test]
    fn parse_errors_yield_multiple_diagnostics_with_the_file_name() {
        let diagnostics = diagnostics(
            "bad.velin",
            "set first\nperform missing(\nlabel still_visible:\n",
        );
        assert_eq!(diagnostics.len(), 2);
        assert!(diagnostics.iter().all(|item| item.file == "bad.velin"));
        assert!(diagnostics.iter().all(Diagnostic::is_error));
    }

    #[test]
    fn completions_include_keywords_builtins_and_declared_names() {
        let items = completions(SCRIPT);
        let has = |label: &str, kind: CompletionKind| {
            items.iter().any(|c| c.label == label && c.kind == kind)
        };
        assert!(has("while", CompletionKind::Keyword));
        assert!(has("len", CompletionKind::Function));
        assert!(has("random", CompletionKind::Function));
        assert!(has("chance", CompletionKind::Function));
        assert!(has("hp", CompletionKind::Variable));
        assert!(has("choice", CompletionKind::Variable));
        assert!(has("start", CompletionKind::Label));
    }

    #[test]
    fn completions_retain_names_around_a_parse_error() {
        let items = completions("set before = 1\nset broken\nlabel after:\n");
        assert!(items.iter().any(|c| c.label == "if"));
        assert!(
            items
                .iter()
                .any(|c| c.label == "before" && c.kind == CompletionKind::Variable)
        );
        assert!(
            items
                .iter()
                .any(|c| c.label == "after" && c.kind == CompletionKind::Label)
        );
    }

    #[test]
    fn document_symbols_list_labels_with_lines() {
        let symbols = document_symbols(SCRIPT);
        let names: Vec<_> = symbols.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, vec!["start", "done"]);
        assert_eq!(symbols[0].line, 2);
    }

    #[test]
    fn names_and_symbols_walk_else_while_and_jump() {
        let source = "\
if true:
    label inner:
        set x = 1
        jump done
else:
    label other:
        set y = 1
while false:
    label looped:
        set z = 1
label done:
    perform say(\"ok\")
";
        let items = completions(source);
        assert!(
            items
                .iter()
                .any(|c| c.label == "inner" && c.kind == CompletionKind::Label)
        );
        assert!(
            items
                .iter()
                .any(|c| c.label == "y" && c.kind == CompletionKind::Variable)
        );
        let names: Vec<_> = document_symbols(source)
            .into_iter()
            .map(|symbol| symbol.name)
            .collect();
        assert!(names.contains(&"inner".to_owned()));
        assert!(names.contains(&"other".to_owned()));
        assert!(names.contains(&"looped".to_owned()));
    }

    #[test]
    fn symbols_survive_a_malformed_block_before_a_valid_label() {
        let source = "while true:\n    set broken\nlabel done:\n";
        assert_eq!(
            document_symbols(source),
            vec![LabelSymbol {
                name: "done".to_owned(),
                line: 3,
            }]
        );
    }

    #[test]
    fn hover_describes_language_names_in_recovered_source() {
        let source = "set score = 1\nset broken\nperform notify(len(score))\n";

        let keyword = hover(source, 1, 2).unwrap();
        assert!(keyword.contents.contains("keyword"));
        assert_eq!(keyword.range.start_column, 1);

        let builtin = hover(source, 3, 17).unwrap();
        assert!(builtin.contents.contains("len(collection)"));

        let variable = hover(source, 3, 21).unwrap();
        assert!(variable.contents.contains("variable"));

        let host = hover(source, 3, 10).unwrap();
        assert!(host.contents.contains("host command"));
        assert!(hover(source, 2, 5).is_none());
    }

    #[test]
    fn label_navigation_survives_an_error_between_references() {
        let source = "label start:\n\
                      \x20\x20\x20\x20jump start\n\
                      \x20\x20\x20\x20set broken\n\
                      \x20\x20\x20\x20jump start\n\
                      label done:\n";

        assert_eq!(
            label_definition(source, 4, 10),
            Some(SourceRange {
                line: 1,
                start_column: 7,
                end_column: 12,
            })
        );
        assert_eq!(label_references(source, 1, 7, true).len(), 3);
        let references = label_references(source, 2, 10, false);
        assert_eq!(references.len(), 2);
        assert!(references.iter().all(|range| range.line > 1));
        assert!(label_definition(source, 5, 7).is_some());
    }
}
