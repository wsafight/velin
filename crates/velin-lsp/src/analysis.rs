//! Language analysis: turning a document's text into the three capabilities
//! this server exposes — diagnostics, completions, and document symbols.
//!
//! Every capability is a thin adaptor over the `velin` library: diagnostics are
//! `compile` + `check_script`, completions merge the fixed keyword/builtin
//! vocabulary with the identifiers a script actually declares, and symbols are
//! the script's labels. The server layer (`server.rs`) only has to translate
//! these into LSP JSON — no language logic lives there.

use velin::{
    Diagnostic, HostCommand, HostSchema, Stmt, check_script, check_script_with_host_schema,
    compile, parse_program_recovering,
};

#[path = "analysis/cursor.rs"]
mod cursor;
#[path = "analysis/navigation.rs"]
mod navigation;
use cursor::{host_call_at, word_at};
use navigation::label_occurrences;
pub use navigation::{label_definition, label_references};

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
    pub detail: String,
    pub documentation: Option<String>,
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

/// Host-call signature information at a source position.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignatureHelp {
    pub label: String,
    pub documentation: Option<String>,
    pub parameters: Vec<String>,
    pub active_parameter: usize,
}

/// Computes syntax, lowering, and static-check diagnostics for `text`.
/// Recoverable syntax errors are all returned in one pass; lowering and static
/// checks only run once the document is syntactically complete.
#[must_use]
pub fn diagnostics(file: &str, text: &str) -> Vec<Diagnostic> {
    diagnostics_inner(file, text, None)
}

/// Computes diagnostics using the host application's command declarations.
#[must_use]
pub fn diagnostics_with_host_schema(
    file: &str,
    text: &str,
    schema: &HostSchema,
) -> Vec<Diagnostic> {
    diagnostics_inner(file, text, Some(schema))
}

fn diagnostics_inner(file: &str, text: &str, schema: Option<&HostSchema>) -> Vec<Diagnostic> {
    let recovered = parse_program_recovering(text);
    if !recovered.errors.is_empty() {
        return recovered
            .errors
            .into_iter()
            .map(|error| error.into_diagnostic(file))
            .collect();
    }
    match compile(file, text) {
        Ok(script) => schema.map_or_else(
            || check_script(file, &script),
            |schema| check_script_with_host_schema(file, &script, schema),
        ),
        Err(diagnostic) => vec![diagnostic],
    }
}

/// Builds the completion list for `text`: the fixed keyword and builtin
/// vocabulary plus every variable and label recoverable from the current text.
#[must_use]
pub fn completions(text: &str) -> Vec<Completion> {
    completions_inner(text, None)
}

/// Builds completions including every command in the supplied host schema.
#[must_use]
pub fn completions_with_host_schema(text: &str, schema: &HostSchema) -> Vec<Completion> {
    completions_inner(text, Some(schema))
}

fn completions_inner(text: &str, schema: Option<&HostSchema>) -> Vec<Completion> {
    let mut items: Vec<Completion> = KEYWORDS
        .iter()
        .map(|keyword| Completion {
            label: (*keyword).to_owned(),
            kind: CompletionKind::Keyword,
            detail: "keyword".to_owned(),
            documentation: None,
        })
        .chain(BUILTINS.iter().map(|name| Completion {
            label: (*name).to_owned(),
            kind: CompletionKind::Function,
            detail: "builtin".to_owned(),
            documentation: None,
        }))
        .collect();

    if let Some(schema) = schema {
        items.extend(schema.commands().map(|command| Completion {
            label: command.name().to_owned(),
            kind: CompletionKind::Function,
            detail: command.signature_label(),
            documentation: command.documentation().map(str::to_owned),
        }));
    }

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
        detail: "variable".to_owned(),
        documentation: None,
    }));
    items.extend(labels.into_iter().map(|name| Completion {
        label: name,
        kind: CompletionKind::Label,
        detail: "label".to_owned(),
        documentation: None,
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
    hover_inner(text, line, column, None)
}

/// Returns hover information enriched by the host application's schema.
#[must_use]
pub fn hover_with_host_schema(
    text: &str,
    line: usize,
    column: usize,
    schema: &HostSchema,
) -> Option<Hover> {
    hover_inner(text, line, column, Some(schema))
}

fn hover_inner(
    text: &str,
    line: usize,
    column: usize,
    schema: Option<&HostSchema>,
) -> Option<Hover> {
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
                    schema
                        .and_then(|schema| schema.command_info(word))
                        .map_or_else(
                            || {
                                format!(
                                    "`{word}` host command\n\nBehavior and return type are supplied by the embedder."
                                )
                            },
                            host_hover,
                        )
                } else {
                    return None;
                }
            }
        }
    };
    Some(Hover { contents, range })
}

/// Returns schema-backed signature help for the host call at the cursor.
#[must_use]
pub fn signature_help(
    text: &str,
    line: usize,
    column: usize,
    schema: &HostSchema,
) -> Option<SignatureHelp> {
    let (name, active_parameter) = host_call_at(text, line, column)?;
    let command = schema.command_info(&name)?;
    let mut parameters: Vec<String> = command
        .signature()
        .arguments()
        .iter()
        .map(|argument| argument.name().to_owned())
        .collect();
    if let Some(variadic) = command.signature().variadic_type() {
        parameters.push(format!("{}...", variadic.name()));
    }
    let active_parameter = active_parameter.min(parameters.len().saturating_sub(1));
    Some(SignatureHelp {
        label: command.signature_label(),
        documentation: command.documentation().map(str::to_owned),
        parameters,
        active_parameter,
    })
}

fn host_hover(command: &HostCommand) -> String {
    let mut contents = format!(
        "```velin\nperform {}\n```\n\nHost command.",
        command.signature_label()
    );
    if let Some(documentation) = command.documentation() {
        contents.push_str("\n\n");
        contents.push_str(documentation);
    }
    contents
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
            Stmt::While { body, .. } | Stmt::For { body, .. } | Stmt::Function { body, .. } => {
                collect_names(body, variables, labels);
            }
            Stmt::Call { bind, .. } => variables.push(bind.clone()),
            Stmt::Jump { .. }
            | Stmt::Import { .. }
            | Stmt::Return { .. }
            | Stmt::Break { .. }
            | Stmt::Continue { .. } => {}
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
            Stmt::While { body, .. } | Stmt::For { body, .. } | Stmt::Function { body, .. } => {
                collect_labels(body, symbols);
            }
            _ => {}
        }
    }
}

fn collect_hosts(statements: &[Stmt], hosts: &mut Vec<String>) {
    for statement in statements {
        match statement {
            Stmt::Label { body, .. }
            | Stmt::While { body, .. }
            | Stmt::For { body, .. }
            | Stmt::Function { body, .. } => collect_hosts(body, hosts),
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
            Stmt::Default { .. }
            | Stmt::Set { .. }
            | Stmt::Jump { .. }
            | Stmt::Import { .. }
            | Stmt::Call { .. }
            | Stmt::Return { .. }
            | Stmt::Break { .. }
            | Stmt::Continue { .. } => {}
        }
    }
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
#[path = "analysis_tests.rs"]
mod tests;
