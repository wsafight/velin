//! LSP capability declarations, workspace diagnostics, and coordinate conversion.

use crate::analysis::{self, CompletionKind, SemanticTokenKind, SymbolKind};
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};
use velin::{Diagnostic, HostSchema};

pub(super) fn is_known_method(method: &str) -> bool {
    matches!(
        method,
        "initialize"
            | "initialized"
            | "shutdown"
            | "textDocument/didOpen"
            | "textDocument/didChange"
            | "textDocument/didClose"
            | "textDocument/completion"
            | "textDocument/documentSymbol"
            | "textDocument/hover"
            | "textDocument/signatureHelp"
            | "textDocument/definition"
            | "textDocument/references"
            | "textDocument/formatting"
            | "textDocument/semanticTokens/full"
            | "textDocument/prepareRename"
            | "textDocument/rename"
            | "textDocument/codeAction"
            | "workspace/symbol"
    )
}

/// The server capabilities advertised in the `initialize` response.
pub(super) fn initialize_result() -> Value {
    json!({
        "capabilities": {
            "textDocumentSync": 1, // full document sync
            "completionProvider": { "triggerCharacters": [] },
            "signatureHelpProvider": { "triggerCharacters": ["(", ","] },
            "documentSymbolProvider": true,
            "hoverProvider": true,
            "definitionProvider": true,
            "referencesProvider": true,
            "documentFormattingProvider": true,
            "renameProvider": { "prepareProvider": true },
            "codeActionProvider": true,
            "workspaceSymbolProvider": true,
            "semanticTokensProvider": {
                "legend": {
                    "tokenTypes": ["keyword", "variable", "function", "string", "number", "operator", "label", "namespace"],
                    "tokenModifiers": [],
                },
                "full": true,
            },
        },
        "serverInfo": { "name": "velin-lsp", "version": env!("CARGO_PKG_VERSION") },
    })
}

pub(super) fn document_diagnostics(
    uri: &str,
    text: &str,
    documents: &HashMap<String, String>,
    schema: Option<&HostSchema>,
) -> Vec<Diagnostic> {
    let module_syntax = analysis::symbols(text)
        .iter()
        .any(|symbol| matches!(symbol.kind, SymbolKind::Module | SymbolKind::Function));
    if !module_syntax {
        return schema.map_or_else(
            || analysis::diagnostics(uri, text),
            |schema| analysis::diagnostics_with_host_schema(uri, text, schema),
        );
    }
    let resolver = |_importer: &str, specifier: &str| {
        find_module_document(documents, specifier)
            .map(|(_, source)| velin::ResolvedModule::new(specifier, source.clone()))
            .ok_or_else(|| "module is not loaded in the workspace".to_owned())
    };
    match velin::compile_modules(uri, text, &resolver) {
        Ok(script) => schema.map_or_else(
            || velin::check_script(uri, &script),
            |schema| velin::check_script_with_host_schema(uri, &script, schema),
        ),
        Err(diagnostic) => vec![diagnostic],
    }
}

pub(super) fn module_diagnostics(
    uri: &str,
    text: &str,
    documents: &HashMap<String, String>,
) -> Vec<Diagnostic> {
    let current = module_name(uri).unwrap_or_default();
    analysis::symbols(text)
        .into_iter()
        .filter(|symbol| symbol.kind == SymbolKind::Module)
        .filter_map(|symbol| {
            if find_module_document(documents, &symbol.name).is_none() {
                return Some(Diagnostic::new(
                    uri,
                    symbol.range.line,
                    symbol.range.start_column,
                    format!("cannot resolve module `{}` in the workspace", symbol.name),
                ));
            }
            let mut visited = HashSet::new();
            module_reaches(documents, &symbol.name, current, &mut visited).then(|| {
                Diagnostic::new(
                    uri,
                    symbol.range.line,
                    symbol.range.start_column,
                    format!(
                        "cyclic module import: {current} -> {} -> {current}",
                        symbol.name
                    ),
                )
            })
        })
        .collect()
}

fn module_reaches(
    documents: &HashMap<String, String>,
    from: &str,
    target: &str,
    visited: &mut HashSet<String>,
) -> bool {
    if from == target {
        return true;
    }
    if !visited.insert(from.to_owned()) {
        return false;
    }
    let Some((_, source)) = find_module_document(documents, from) else {
        return false;
    };
    analysis::symbols(source)
        .into_iter()
        .filter(|symbol| symbol.kind == SymbolKind::Module)
        .any(|symbol| module_reaches(documents, &symbol.name, target, visited))
}

fn find_module_document<'a>(
    documents: &'a HashMap<String, String>,
    name: &str,
) -> Option<(&'a String, &'a String)> {
    documents
        .iter()
        .find(|(uri, _)| module_name(uri).is_some_and(|module| module == name))
}

fn module_name(uri: &str) -> Option<&str> {
    uri.rsplit('/')
        .next()?
        .strip_suffix(".velin")
        .filter(|name| !name.is_empty())
}

pub(super) fn full_document_edit(text: &str, formatted: &str) -> Value {
    let line_count = text.lines().count();
    let (end_line, end_character) = if text.ends_with('\n') {
        (line_count, 0)
    } else {
        let last = text.lines().next_back().unwrap_or_default();
        (line_count.saturating_sub(1), last.encode_utf16().count())
    };
    json!({
        "range": {
            "start": { "line": 0, "character": 0 },
            "end": { "line": end_line, "character": end_character },
        },
        "newText": formatted,
    })
}

pub(super) fn semantic_token_data(text: &str) -> Vec<u32> {
    let lines: Vec<&str> = text.lines().collect();
    let mut data = Vec::new();
    let mut previous_line = 0usize;
    let mut previous_start = 0usize;
    for token in analysis::semantic_tokens(text) {
        let line = token.range.line.saturating_sub(1);
        let source_line = lines.get(line).copied().unwrap_or_default();
        let start: usize = source_line
            .chars()
            .take(token.range.start_column.saturating_sub(1))
            .map(char::len_utf16)
            .sum();
        let length: usize = source_line
            .chars()
            .skip(token.range.start_column.saturating_sub(1))
            .take(
                token
                    .range
                    .end_column
                    .saturating_sub(token.range.start_column),
            )
            .map(char::len_utf16)
            .sum();
        let delta_line = line.saturating_sub(previous_line);
        let delta_start = if delta_line == 0 {
            start.saturating_sub(previous_start)
        } else {
            start
        };
        data.extend([
            u32::try_from(delta_line).unwrap_or(u32::MAX),
            u32::try_from(delta_start).unwrap_or(u32::MAX),
            u32::try_from(length).unwrap_or(u32::MAX),
            semantic_token_kind(token.kind),
            0,
        ]);
        previous_line = line;
        previous_start = start;
    }
    data
}

pub(super) const fn semantic_token_kind(kind: SemanticTokenKind) -> u32 {
    match kind {
        SemanticTokenKind::Keyword => 0,
        SemanticTokenKind::Variable => 1,
        SemanticTokenKind::Function => 2,
        SemanticTokenKind::String => 3,
        SemanticTokenKind::Number => 4,
        SemanticTokenKind::Operator => 5,
        SemanticTokenKind::Label => 6,
        SemanticTokenKind::Namespace => 7,
    }
}

pub(super) const fn symbol_kind(kind: SymbolKind) -> u8 {
    match kind {
        SymbolKind::Label | SymbolKind::Variable => 13,
        SymbolKind::Function => 12,
        SymbolKind::Module => 2,
    }
}

pub(super) fn workspace_symbol_values(
    documents: &HashMap<String, String>,
    message: &Value,
) -> Vec<Value> {
    let query = message["params"]["query"]
        .as_str()
        .unwrap_or_default()
        .to_ascii_lowercase();
    let mut output = Vec::new();
    for (uri, text) in documents {
        let lines: Vec<&str> = text.lines().collect();
        for symbol in analysis::symbols(text) {
            if !query.is_empty() && !symbol.name.to_ascii_lowercase().contains(&query) {
                continue;
            }
            output.push(json!({
                "name": symbol.name,
                "kind": symbol_kind(symbol.kind),
                "location": { "uri": uri, "range": lsp_source_range(symbol.range, &lines) },
            }));
        }
    }
    output
}

pub(super) fn workspace_occurrences(
    documents: &HashMap<String, String>,
    name: &str,
    include_declaration: bool,
) -> Vec<Value> {
    let mut output = Vec::new();
    for (uri, text) in documents {
        let declarations = analysis::symbols(text);
        let lines: Vec<&str> = text.lines().collect();
        for range in analysis::identifier_occurrences(text, name) {
            if !include_declaration
                && declarations
                    .iter()
                    .any(|symbol| symbol.name == name && symbol.range == range)
            {
                continue;
            }
            output.push(json!({ "uri": uri, "range": lsp_source_range(range, &lines) }));
        }
    }
    output
}

/// Converts a Velin [`Diagnostic`] (1-based Unicode-scalar line/column) into an
/// LSP diagnostic (0-based UTF-16 positions, numeric severity).
pub(super) fn lsp_diagnostic(diagnostic: &Diagnostic, lines: &[&str]) -> Value {
    let line = diagnostic.line.saturating_sub(1);
    let scalar_offset = diagnostic.column.saturating_sub(1);
    let source_line = lines.get(line).copied().unwrap_or_default();
    let character: usize = source_line
        .chars()
        .take(scalar_offset)
        .map(char::len_utf16)
        .sum();
    let width = source_line
        .chars()
        .nth(scalar_offset)
        .map_or(0, char::len_utf16);
    let mut message = diagnostic.message.clone();
    if let Some(hint) = &diagnostic.hint {
        message.push_str("\nhint: ");
        message.push_str(hint);
    }
    json!({
        "range": {
            "start": { "line": line, "character": character },
            "end": { "line": line, "character": character + width },
        },
        "severity": if diagnostic.is_error() { 1 } else { 2 },
        "source": "velin",
        "message": message,
    })
}

/// A whole-line LSP range for a 1-based `line`, used for label symbols.
pub(super) fn line_range(line: usize, lines: &[&str]) -> Value {
    let zero = line.saturating_sub(1);
    let width: usize = lines
        .get(zero)
        .copied()
        .unwrap_or_default()
        .chars()
        .map(char::len_utf16)
        .sum();
    json!({
        "start": { "line": zero, "character": 0 },
        "end": { "line": zero, "character": width },
    })
}

pub(super) fn document_uri(message: &Value) -> Option<&str> {
    message["params"]["textDocument"]
        .get("uri")
        .and_then(Value::as_str)
}

/// Converts an LSP UTF-16 position to Velin's 1-based Unicode-scalar position.
pub(super) fn request_position(message: &Value, text: &str) -> Option<(usize, usize)> {
    let line = usize::try_from(message["params"]["position"]["line"].as_u64()?).ok()?;
    let requested = usize::try_from(message["params"]["position"]["character"].as_u64()?).ok()?;
    let source_line = text.lines().nth(line)?;
    let mut utf16 = 0;
    let mut scalars = 0;
    for character in source_line.chars() {
        if utf16 >= requested {
            break;
        }
        let width = character.len_utf16();
        if utf16 + width > requested {
            break;
        }
        utf16 += width;
        scalars += 1;
    }
    if requested > utf16 && utf16 == source_line.encode_utf16().count() {
        return None;
    }
    Some((line + 1, scalars + 1))
}

pub(super) fn lsp_source_range(range: analysis::SourceRange, lines: &[&str]) -> Value {
    let line = range.line.saturating_sub(1);
    let source_line = lines.get(line).copied().unwrap_or_default();
    let start: usize = source_line
        .chars()
        .take(range.start_column.saturating_sub(1))
        .map(char::len_utf16)
        .sum();
    let end: usize = source_line
        .chars()
        .take(range.end_column.saturating_sub(1))
        .map(char::len_utf16)
        .sum();
    json!({
        "start": { "line": line, "character": start },
        "end": { "line": line, "character": end },
    })
}

/// Maps our [`CompletionKind`] to the LSP `CompletionItemKind` numbers.
pub(super) fn completion_item_kind(kind: CompletionKind) -> u8 {
    match kind {
        CompletionKind::Keyword => 14, // Keyword
        CompletionKind::Function => 3, // Function
        CompletionKind::Variable => 6, // Variable
        CompletionKind::Label => 12,   // Value (closest for a jump target)
    }
}
