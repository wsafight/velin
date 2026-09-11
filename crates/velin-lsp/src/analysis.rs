//! Language analysis: turning a document's text into the three capabilities
//! this server exposes — diagnostics, completions, and document symbols.
//!
//! Every capability is a thin adaptor over the `velin` library: diagnostics are
//! `compile` + `check_script`, completions merge the fixed keyword/builtin
//! vocabulary with the identifiers a script actually declares, and symbols are
//! the script's labels. The server layer (`server.rs`) only has to translate
//! these into LSP JSON — no language logic lives there.

use velin::{Diagnostic, Stmt, check_script, compile, parse_program};

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

/// Computes the diagnostics for `text`: a single compile error if it fails to
/// parse or lower, otherwise the static-check findings (which may be empty).
#[must_use]
pub fn diagnostics(file: &str, text: &str) -> Vec<Diagnostic> {
    match compile(file, text) {
        Ok(script) => check_script(file, &script),
        Err(diagnostic) => vec![diagnostic],
    }
}

/// Builds the completion list for `text`: the fixed keyword and builtin
/// vocabulary plus every variable and label the script declares. Declared names
/// are only available when the document currently parses; the fixed vocabulary
/// is always offered so completion still helps while the file is mid-edit.
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

    if let Ok(program) = parse_program(text) {
        let mut variables = Vec::new();
        let mut labels = Vec::new();
        collect_names(&program, &mut variables, &mut labels);
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
    }

    items
}

/// Extracts the label outline for `text` (empty when the file does not parse).
#[must_use]
pub fn document_symbols(text: &str) -> Vec<LabelSymbol> {
    let Ok(program) = parse_program(text) else {
        return Vec::new();
    };
    let mut symbols = Vec::new();
    collect_labels(&program, &mut symbols);
    symbols
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
    fn a_parse_error_yields_one_diagnostic_with_the_file_name() {
        let diagnostics = diagnostics("bad.velin", "if hp > 0\n");
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].file, "bad.velin");
        assert!(diagnostics[0].is_error());
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
    fn completions_still_offer_the_fixed_vocabulary_when_parsing_fails() {
        let items = completions("if hp > 0\n"); // no `:` → parse error
        assert!(items.iter().any(|c| c.label == "if"));
        assert!(items.iter().all(|c| c.kind != CompletionKind::Variable));
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
        assert!(document_symbols("if hp > 0\n").is_empty());
    }
}
