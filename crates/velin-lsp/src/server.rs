//! The LSP server: a synchronous request/notification loop over the stdio
//! transport in [`crate::protocol`], backed by an in-memory document store.
//!
//! It advertises full-text sync, completion, document symbols, hover, and label
//! navigation, and pushes diagnostics on every open/change. Every language
//! decision is delegated to [`crate::analysis`]; this module only speaks
//! JSON-RPC and converts between Velin's 1-based line/column diagnostics and
//! LSP's 0-based positions.

use crate::analysis;
use crate::protocol::{read_message, write_message};
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};
use std::io::{BufRead, Write};
#[cfg(test)]
use velin::Diagnostic;
use velin::HostSchema;

mod support;
mod workspace;
use support::{
    completion_item_kind, document_diagnostics, document_uri, full_document_edit,
    initialize_result, is_known_method, line_range, lsp_diagnostic, lsp_source_range,
    module_diagnostics, request_position, semantic_token_data, workspace_occurrences,
    workspace_symbol_values,
};

/// A running server instance: the open documents keyed by URI, plus the output
/// stream diagnostics and responses are written to.
pub struct Server<W: Write> {
    documents: HashMap<String, String>,
    workspace_uris: HashSet<String>,
    output: W,
    initialized: bool,
    shutdown: bool,
    host_schema: Option<HostSchema>,
}

impl<W: Write> Server<W> {
    /// Creates a server that writes protocol messages to `output`.
    pub fn new(output: W) -> Self {
        Self {
            documents: HashMap::new(),
            workspace_uris: HashSet::new(),
            output,
            initialized: false,
            shutdown: false,
            host_schema: None,
        }
    }

    /// Creates a server whose diagnostics and editor features share a host schema.
    pub fn with_host_schema(output: W, host_schema: HostSchema) -> Self {
        Self {
            documents: HashMap::new(),
            workspace_uris: HashSet::new(),
            output,
            initialized: false,
            shutdown: false,
            host_schema: Some(host_schema),
        }
    }

    /// Runs the message loop until the client closes the input or sends `exit`.
    ///
    /// # Errors
    /// Propagates any I/O error from reading or writing a protocol message.
    pub fn run(&mut self, input: &mut impl BufRead) -> std::io::Result<()> {
        while let Some(message) = read_message(input)? {
            let method = message.get("method").and_then(Value::as_str).unwrap_or("");
            if method == "exit" {
                if !self.shutdown {
                    return Err(std::io::Error::other("LSP exit received before shutdown"));
                }
                break;
            }
            // `shutdown` is answered by `dispatch` like any other request; the
            // client follows it with `exit`, which ends the loop above.
            self.dispatch(method, &message)?;
        }
        Ok(())
    }

    /// Routes one message to its handler by method name.
    fn dispatch(&mut self, method: &str, message: &Value) -> std::io::Result<()> {
        if self.shutdown && method != "exit" && is_known_method(method) {
            if message.get("id").is_some() {
                return self.respond_error(message, -32600, "server has been shut down");
            }
            return Ok(());
        }
        match method {
            "initialize" if self.initialized => {
                self.respond_error(message, -32600, "server is already initialized")
            }
            "initialize" => {
                let workspace = workspace::load_documents(message);
                self.workspace_uris.extend(workspace.keys().cloned());
                self.documents.extend(workspace);
                self.initialized = true;
                self.respond(message, &initialize_result())
            }
            "shutdown" => {
                self.shutdown = true;
                self.respond(message, &Value::Null)
            }
            "textDocument/didOpen" => self.did_open(message),
            "textDocument/didChange" => self.did_change(message),
            "textDocument/didClose" => {
                self.did_close(message);
                Ok(())
            }
            "textDocument/completion" => self.completion(message),
            "textDocument/documentSymbol" => self.document_symbol(message),
            "textDocument/hover" => self.hover(message),
            "textDocument/signatureHelp" => self.signature_help(message),
            "textDocument/definition" => self.definition(message),
            "textDocument/references" => self.references(message),
            "textDocument/formatting" => self.formatting(message),
            "textDocument/semanticTokens/full" => self.semantic_tokens(message),
            "textDocument/prepareRename" => self.prepare_rename(message),
            "textDocument/rename" => self.rename(message),
            "textDocument/codeAction" => self.code_actions(message),
            "workspace/symbol" => self.workspace_symbols(message),
            // Unknown notifications are ignored; requests receive the JSON-RPC
            // MethodNotFound error required by the protocol.
            _ if message.get("id").is_some() => {
                self.respond_error(message, -32601, &format!("method not found: {method}"))
            }
            _ => Ok(()),
        }
    }

    /// Stores a freshly opened document and publishes its diagnostics.
    fn did_open(&mut self, message: &Value) -> std::io::Result<()> {
        let doc = &message["params"]["textDocument"];
        let (Some(uri), Some(text)) = (
            doc.get("uri").and_then(Value::as_str),
            doc.get("text").and_then(Value::as_str),
        ) else {
            return Ok(());
        };
        self.documents.insert(uri.to_owned(), text.to_owned());
        self.publish_all_diagnostics()
    }

    /// Applies a full-text change (the only sync mode we advertise) and
    /// re-publishes diagnostics.
    fn did_change(&mut self, message: &Value) -> std::io::Result<()> {
        let params = &message["params"];
        let Some(uri) = params["textDocument"].get("uri").and_then(Value::as_str) else {
            return Ok(());
        };
        // Full sync sends the whole document as the last change's `text`.
        if let Some(text) = params["contentChanges"]
            .as_array()
            .and_then(|changes| changes.last())
            .and_then(|change| change.get("text"))
            .and_then(Value::as_str)
        {
            self.documents.insert(uri.to_owned(), text.to_owned());
        }
        self.publish_all_diagnostics()
    }

    /// Forgets a closed document and clears its diagnostics.
    fn did_close(&mut self, message: &Value) {
        if let Some(uri) = message["params"]["textDocument"]
            .get("uri")
            .and_then(Value::as_str)
        {
            workspace::restore_closed_document(uri, &mut self.documents, &mut self.workspace_uris);
            let cleared = json!({
                "jsonrpc": "2.0",
                "method": "textDocument/publishDiagnostics",
                "params": { "uri": uri, "diagnostics": [] },
            });
            let _ = write_message(&mut self.output, &cleared);
            let _ = self.publish_all_diagnostics();
        }
    }

    /// Answers a completion request with the merged vocabulary + declared names.
    fn completion(&mut self, message: &Value) -> std::io::Result<()> {
        let text = self.document_for(message).unwrap_or_default();
        let completions = self.host_schema.as_ref().map_or_else(
            || analysis::completions(text),
            |schema| analysis::completions_with_host_schema(text, schema),
        );
        let items: Vec<Value> = completions
            .into_iter()
            .map(|completion| {
                json!({
                    "label": completion.label,
                    "kind": completion_item_kind(completion.kind),
                    "detail": completion.detail,
                    "documentation": completion.documentation,
                })
            })
            .collect();
        self.respond(message, &Value::Array(items))
    }

    /// Answers a document-symbol request with the script's labels.
    fn document_symbol(&mut self, message: &Value) -> std::io::Result<()> {
        let text = self.document_for(message).unwrap_or_default();
        let lines: Vec<&str> = text.lines().collect();
        let symbols: Vec<Value> = analysis::document_symbols(text)
            .into_iter()
            .map(|symbol| {
                let range = line_range(symbol.line, &lines);
                json!({
                    "name": symbol.name,
                    "kind": 12, // SymbolKind.Function — the closest LSP kind for a jump target.
                    "range": range,
                    "selectionRange": range,
                })
            })
            .collect();
        self.respond(message, &Value::Array(symbols))
    }

    fn hover(&mut self, message: &Value) -> std::io::Result<()> {
        let result = self.document_for(message).and_then(|text| {
            let (line, column) = request_position(message, text)?;
            let lines: Vec<&str> = text.lines().collect();
            let hover = self.host_schema.as_ref().map_or_else(
                || analysis::hover(text, line, column),
                |schema| analysis::hover_with_host_schema(text, line, column, schema),
            )?;
            Some(json!({
                "contents": { "kind": "markdown", "value": hover.contents },
                "range": lsp_source_range(hover.range, &lines),
            }))
        });
        self.respond(message, &result.unwrap_or(Value::Null))
    }

    fn signature_help(&mut self, message: &Value) -> std::io::Result<()> {
        let result = self.document_for(message).and_then(|text| {
            let schema = self.host_schema.as_ref()?;
            let (line, column) = request_position(message, text)?;
            let help = analysis::signature_help(text, line, column, schema)?;
            let parameters: Vec<Value> = help
                .parameters
                .into_iter()
                .map(|label| json!({ "label": label }))
                .collect();
            Some(json!({
                "signatures": [{
                    "label": help.label,
                    "documentation": help.documentation,
                    "parameters": parameters,
                }],
                "activeSignature": 0,
                "activeParameter": help.active_parameter,
            }))
        });
        self.respond(message, &result.unwrap_or(Value::Null))
    }

    fn definition(&mut self, message: &Value) -> std::io::Result<()> {
        let result = self.document_for(message).and_then(|text| {
            let uri = document_uri(message)?;
            let (line, column) = request_position(message, text)?;
            let lines: Vec<&str> = text.lines().collect();
            if let Some(range) = analysis::label_definition(text, line, column) {
                return Some(json!({ "uri": uri, "range": lsp_source_range(range, &lines) }));
            }
            let (name, _) = analysis::identifier_at(text, line, column)?;
            if let Some(range) = analysis::definition(text, line, column) {
                return Some(json!({ "uri": uri, "range": lsp_source_range(range, &lines) }));
            }
            self.documents.iter().find_map(|(target_uri, target)| {
                let symbol = analysis::symbols(target)
                    .into_iter()
                    .find(|symbol| symbol.name == name)?;
                let target_lines: Vec<&str> = target.lines().collect();
                Some(json!({
                    "uri": target_uri,
                    "range": lsp_source_range(symbol.range, &target_lines),
                }))
            })
        });
        self.respond(message, &result.unwrap_or(Value::Null))
    }

    fn references(&mut self, message: &Value) -> std::io::Result<()> {
        let result = if let Some(text) = self.document_for(message) {
            let position = request_position(message, text);
            let include_declaration = message["params"]["context"]["includeDeclaration"]
                .as_bool()
                .unwrap_or(true);
            position.map_or_else(Vec::new, |(line, column)| {
                let Some((name, _)) = analysis::identifier_at(text, line, column) else {
                    return Vec::new();
                };
                workspace_occurrences(&self.documents, &name, include_declaration)
            })
        } else {
            Vec::new()
        };
        self.respond(message, &Value::Array(result))
    }

    fn formatting(&mut self, message: &Value) -> std::io::Result<()> {
        let edits = self.document_for(message).map_or_else(Vec::new, |text| {
            velin::format_source(text).map_or_else(
                |_| Vec::new(),
                |formatted| {
                    if formatted == text {
                        Vec::new()
                    } else {
                        vec![full_document_edit(text, &formatted)]
                    }
                },
            )
        });
        self.respond(message, &Value::Array(edits))
    }

    fn semantic_tokens(&mut self, message: &Value) -> std::io::Result<()> {
        let data = self
            .document_for(message)
            .map(semantic_token_data)
            .unwrap_or_default();
        self.respond(message, &json!({ "data": data }))
    }

    fn prepare_rename(&mut self, message: &Value) -> std::io::Result<()> {
        let result = self.document_for(message).and_then(|text| {
            let (line, column) = request_position(message, text)?;
            let (name, range) = analysis::identifier_at(text, line, column)?;
            analysis::valid_rename(&name).then(|| {
                let lines: Vec<&str> = text.lines().collect();
                json!({ "range": lsp_source_range(range, &lines), "placeholder": name })
            })
        });
        self.respond(message, &result.unwrap_or(Value::Null))
    }

    fn rename(&mut self, message: &Value) -> std::io::Result<()> {
        let new_name = message["params"]["newName"].as_str().unwrap_or_default();
        if !analysis::valid_rename(new_name) {
            return self.respond_error(message, -32602, "new name is not a valid Velin identifier");
        }
        let Some(text) = self.document_for(message) else {
            return self.respond(message, &Value::Null);
        };
        let Some((line, column)) = request_position(message, text) else {
            return self.respond(message, &Value::Null);
        };
        let Some((name, _)) = analysis::identifier_at(text, line, column) else {
            return self.respond(message, &Value::Null);
        };
        if !analysis::valid_rename(&name) {
            return self.respond(message, &Value::Null);
        }
        let mut changes = serde_json::Map::new();
        for (uri, source) in &self.documents {
            let lines: Vec<&str> = source.lines().collect();
            let edits: Vec<Value> = analysis::identifier_occurrences(source, &name)
                .into_iter()
                .map(|range| {
                    json!({
                        "range": lsp_source_range(range, &lines),
                        "newText": new_name,
                    })
                })
                .collect();
            if !edits.is_empty() {
                changes.insert(uri.clone(), Value::Array(edits));
            }
        }
        self.respond(message, &json!({ "changes": changes }))
    }

    fn code_actions(&mut self, message: &Value) -> std::io::Result<()> {
        let actions = self.document_for(message).map_or_else(Vec::new, |text| {
            let uri = document_uri(message).unwrap_or_default();
            velin::format_source(text).map_or_else(
                |_| Vec::new(),
                |formatted| {
                    if formatted == text {
                        return Vec::new();
                    }
                    let mut changes = serde_json::Map::new();
                    changes.insert(
                        uri.to_owned(),
                        Value::Array(vec![full_document_edit(text, &formatted)]),
                    );
                    vec![json!({
                        "title": "Format Velin document",
                        "kind": "source.fixAll.velin",
                        "edit": { "changes": changes },
                    })]
                },
            )
        });
        self.respond(message, &Value::Array(actions))
    }

    fn workspace_symbols(&mut self, message: &Value) -> std::io::Result<()> {
        self.respond(
            message,
            &Value::Array(workspace_symbol_values(&self.documents, message)),
        )
    }

    /// Runs diagnostics for `uri`'s current text and pushes them to the client.
    fn publish_diagnostics(&mut self, uri: &str) -> std::io::Result<()> {
        let Some(text) = self.documents.get(uri) else {
            return Ok(());
        };
        let lines: Vec<&str> = text.lines().collect();
        let mut analyzed =
            document_diagnostics(uri, text, &self.documents, self.host_schema.as_ref());
        for diagnostic in module_diagnostics(uri, text, &self.documents) {
            if !analyzed
                .iter()
                .any(|existing| existing.message == diagnostic.message)
            {
                analyzed.push(diagnostic);
            }
        }
        let diagnostics: Vec<Value> = analyzed
            .iter()
            .map(|diagnostic| lsp_diagnostic(diagnostic, &lines))
            .collect();
        let notification = json!({
            "jsonrpc": "2.0",
            "method": "textDocument/publishDiagnostics",
            "params": { "uri": uri, "diagnostics": diagnostics },
        });
        write_message(&mut self.output, &notification)
    }

    fn publish_all_diagnostics(&mut self) -> std::io::Result<()> {
        let uris: Vec<String> = self.documents.keys().cloned().collect();
        for uri in uris {
            self.publish_diagnostics(&uri)?;
        }
        Ok(())
    }

    /// Looks up the document a request targets by its `textDocument.uri`.
    fn document_for<'a>(&'a self, message: &Value) -> Option<&'a str> {
        let uri = message["params"]["textDocument"]
            .get("uri")
            .and_then(Value::as_str)?;
        self.documents.get(uri).map(String::as_str)
    }

    /// Writes a JSON-RPC response echoing the request's `id`.
    fn respond(&mut self, request: &Value, result: &Value) -> std::io::Result<()> {
        let response = json!({
            "jsonrpc": "2.0",
            "id": request.get("id").cloned().unwrap_or(Value::Null),
            "result": result,
        });
        write_message(&mut self.output, &response)
    }

    fn respond_error(&mut self, request: &Value, code: i32, message: &str) -> std::io::Result<()> {
        let response = json!({
            "jsonrpc": "2.0",
            "id": request.get("id").cloned().unwrap_or(Value::Null),
            "error": { "code": code, "message": message },
        });
        write_message(&mut self.output, &response)
    }
}

#[cfg(test)]
#[path = "server_tests.rs"]
mod tests;
#[cfg(test)]
#[path = "server/workspace_tests.rs"]
mod workspace_tests;
