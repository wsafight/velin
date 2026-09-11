//! The LSP server: a synchronous request/notification loop over the stdio
//! transport in [`crate::protocol`], backed by an in-memory document store.
//!
//! It advertises full-text sync, completion, document symbols, hover, and label
//! navigation, and pushes diagnostics on every open/change. Every language
//! decision is delegated to [`crate::analysis`]; this module only speaks
//! JSON-RPC and converts between Velin's 1-based line/column diagnostics and
//! LSP's 0-based positions.

use crate::analysis::{self, CompletionKind};
use crate::protocol::{read_message, write_message};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::io::{BufRead, Write};
use velin::Diagnostic;

/// A running server instance: the open documents keyed by URI, plus the output
/// stream diagnostics and responses are written to.
pub struct Server<W: Write> {
    documents: HashMap<String, String>,
    output: W,
}

impl<W: Write> Server<W> {
    /// Creates a server that writes protocol messages to `output`.
    pub fn new(output: W) -> Self {
        Self {
            documents: HashMap::new(),
            output,
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
        match method {
            "initialize" => self.respond(message, &initialize_result()),
            "shutdown" => self.respond(message, &Value::Null),
            "textDocument/didOpen" => self.did_open(message),
            "textDocument/didChange" => self.did_change(message),
            "textDocument/didClose" => {
                self.did_close(message);
                Ok(())
            }
            "textDocument/completion" => self.completion(message),
            "textDocument/documentSymbol" => self.document_symbol(message),
            "textDocument/hover" => self.hover(message),
            "textDocument/definition" => self.definition(message),
            "textDocument/references" => self.references(message),
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
        self.publish_diagnostics(uri)
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
        self.publish_diagnostics(uri)
    }

    /// Forgets a closed document and clears its diagnostics.
    fn did_close(&mut self, message: &Value) {
        if let Some(uri) = message["params"]["textDocument"]
            .get("uri")
            .and_then(Value::as_str)
        {
            self.documents.remove(uri);
            let cleared = json!({
                "jsonrpc": "2.0",
                "method": "textDocument/publishDiagnostics",
                "params": { "uri": uri, "diagnostics": [] },
            });
            let _ = write_message(&mut self.output, &cleared);
        }
    }

    /// Answers a completion request with the merged vocabulary + declared names.
    fn completion(&mut self, message: &Value) -> std::io::Result<()> {
        let text = self.document_for(message).unwrap_or_default();
        let items: Vec<Value> = analysis::completions(text)
            .into_iter()
            .map(|completion| {
                json!({
                    "label": completion.label,
                    "kind": completion_item_kind(completion.kind),
                    "detail": completion.detail,
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
            let hover = analysis::hover(text, line, column)?;
            Some(json!({
                "contents": { "kind": "markdown", "value": hover.contents },
                "range": lsp_source_range(hover.range, &lines),
            }))
        });
        self.respond(message, &result.unwrap_or(Value::Null))
    }

    fn definition(&mut self, message: &Value) -> std::io::Result<()> {
        let result = self.document_for(message).and_then(|text| {
            let uri = document_uri(message)?;
            let (line, column) = request_position(message, text)?;
            let lines: Vec<&str> = text.lines().collect();
            let range = analysis::label_definition(text, line, column)?;
            Some(json!({ "uri": uri, "range": lsp_source_range(range, &lines) }))
        });
        self.respond(message, &result.unwrap_or(Value::Null))
    }

    fn references(&mut self, message: &Value) -> std::io::Result<()> {
        let result = if let Some(text) = self.document_for(message) {
            let uri = document_uri(message).unwrap_or_default();
            let position = request_position(message, text);
            let include_declaration = message["params"]["context"]["includeDeclaration"]
                .as_bool()
                .unwrap_or(true);
            let lines: Vec<&str> = text.lines().collect();
            position.map_or_else(Vec::new, |(line, column)| {
                analysis::label_references(text, line, column, include_declaration)
                    .into_iter()
                    .map(|range| json!({ "uri": uri, "range": lsp_source_range(range, &lines) }))
                    .collect()
            })
        } else {
            Vec::new()
        };
        self.respond(message, &Value::Array(result))
    }

    /// Runs diagnostics for `uri`'s current text and pushes them to the client.
    fn publish_diagnostics(&mut self, uri: &str) -> std::io::Result<()> {
        let Some(text) = self.documents.get(uri) else {
            return Ok(());
        };
        let lines: Vec<&str> = text.lines().collect();
        let diagnostics: Vec<Value> = analysis::diagnostics(uri, text)
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

/// The server capabilities advertised in the `initialize` response.
fn initialize_result() -> Value {
    json!({
        "capabilities": {
            "textDocumentSync": 1, // full document sync
            "completionProvider": { "triggerCharacters": [] },
            "documentSymbolProvider": true,
            "hoverProvider": true,
            "definitionProvider": true,
            "referencesProvider": true,
        },
        "serverInfo": { "name": "velin-lsp", "version": env!("CARGO_PKG_VERSION") },
    })
}

/// Converts a Velin [`Diagnostic`] (1-based Unicode-scalar line/column) into an
/// LSP diagnostic (0-based UTF-16 positions, numeric severity).
fn lsp_diagnostic(diagnostic: &Diagnostic, lines: &[&str]) -> Value {
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
fn line_range(line: usize, lines: &[&str]) -> Value {
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

fn document_uri(message: &Value) -> Option<&str> {
    message["params"]["textDocument"]
        .get("uri")
        .and_then(Value::as_str)
}

/// Converts an LSP UTF-16 position to Velin's 1-based Unicode-scalar position.
fn request_position(message: &Value, text: &str) -> Option<(usize, usize)> {
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

fn lsp_source_range(range: analysis::SourceRange, lines: &[&str]) -> Value {
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
fn completion_item_kind(kind: CompletionKind) -> u8 {
    match kind {
        CompletionKind::Keyword => 14, // Keyword
        CompletionKind::Function => 3, // Function
        CompletionKind::Variable => 6, // Variable
        CompletionKind::Label => 12,   // Value (closest for a jump target)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::write_message;
    use serde_json::json;
    use std::io::Cursor;

    /// Frames a batch of messages into a single input stream.
    fn stream(messages: &[Value]) -> Cursor<Vec<u8>> {
        let mut buffer = Vec::new();
        for message in messages {
            write_message(&mut buffer, message).unwrap();
        }
        Cursor::new(buffer)
    }

    /// Splits a captured output stream back into individual JSON messages.
    fn parse_all(bytes: Vec<u8>) -> Vec<Value> {
        let mut cursor = Cursor::new(bytes);
        let mut out = Vec::new();
        while let Some(message) = read_message(&mut cursor).unwrap() {
            out.push(message);
        }
        out
    }

    #[test]
    fn initialize_then_open_publishes_diagnostics() {
        let mut input = stream(&[
            json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {} }),
            json!({
                "jsonrpc": "2.0",
                "method": "textDocument/didOpen",
                "params": { "textDocument": {
                    "uri": "file:///bad.velin",
                    "text": "if hp > 0\n",
                } },
            }),
            json!({ "jsonrpc": "2.0", "method": "exit" }),
        ]);
        let mut output = Vec::new();
        Server::new(&mut output).run(&mut input).unwrap();

        let messages = parse_all(output);
        // First message answers initialize with capabilities.
        assert!(messages[0]["result"]["capabilities"]["completionProvider"].is_object());
        assert_eq!(messages[0]["result"]["capabilities"]["hoverProvider"], true);
        assert_eq!(
            messages[0]["result"]["capabilities"]["definitionProvider"],
            true
        );
        // Second is a diagnostics push for the parse error.
        let diags = &messages[1]["params"]["diagnostics"];
        assert_eq!(messages[1]["method"], "textDocument/publishDiagnostics");
        assert_eq!(diags.as_array().unwrap().len(), 1);
        assert_eq!(diags[0]["severity"], 1);
    }

    #[test]
    fn diagnostics_are_converted_to_utf16_positions() {
        let diagnostic = Diagnostic::new("test.velin", 1, 3, "bad token");
        let converted = lsp_diagnostic(&diagnostic, &["\u{4f60}\u{1f600}+"]);

        assert_eq!(converted["range"]["start"]["character"], 3);
        assert_eq!(converted["range"]["end"]["character"], 4);

        let eof = Diagnostic::new("test.velin", 1, 4, "expected value");
        let converted = lsp_diagnostic(&eof, &["\u{4f60}\u{1f600}+"]);
        assert_eq!(converted["range"]["start"]["character"], 4);
        assert_eq!(converted["range"]["end"]["character"], 4);
    }

    #[test]
    fn change_reruns_diagnostics_and_completion_sees_new_names() {
        let mut input = stream(&[
            json!({
                "jsonrpc": "2.0",
                "method": "textDocument/didOpen",
                "params": { "textDocument": { "uri": "file:///a.velin", "text": "" } },
            }),
            json!({
                "jsonrpc": "2.0",
                "method": "textDocument/didChange",
                "params": {
                    "textDocument": { "uri": "file:///a.velin" },
                    "contentChanges": [{ "text": "default score = 5\nlabel start:\n" }],
                },
            }),
            json!({
                "jsonrpc": "2.0",
                "id": 2,
                "method": "textDocument/completion",
                "params": { "textDocument": { "uri": "file:///a.velin" } },
            }),
            json!({ "jsonrpc": "2.0", "method": "exit" }),
        ]);
        let mut output = Vec::new();
        Server::new(&mut output).run(&mut input).unwrap();

        let messages = parse_all(output);
        let completion = messages
            .iter()
            .find(|m| m.get("id") == Some(&json!(2)))
            .expect("completion response");
        let labels: Vec<&str> = completion["result"]
            .as_array()
            .unwrap()
            .iter()
            .map(|item| item["label"].as_str().unwrap())
            .collect();
        assert!(labels.contains(&"score"));
        assert!(labels.contains(&"start"));
        assert!(labels.contains(&"perform"));
    }

    #[test]
    fn document_symbol_lists_labels() {
        let mut input = stream(&[
            json!({
                "jsonrpc": "2.0",
                "method": "textDocument/didOpen",
                "params": { "textDocument": {
                    "uri": "file:///s.velin",
                    "text": "label start:\n    perform say(\"hi\")\nlabel done:\n    perform say(\"bye\")\n",
                } },
            }),
            json!({
                "jsonrpc": "2.0",
                "id": 3,
                "method": "textDocument/documentSymbol",
                "params": { "textDocument": { "uri": "file:///s.velin" } },
            }),
            json!({ "jsonrpc": "2.0", "method": "exit" }),
        ]);
        let mut output = Vec::new();
        Server::new(&mut output).run(&mut input).unwrap();

        let messages = parse_all(output);
        let symbols = messages
            .iter()
            .find(|m| m.get("id") == Some(&json!(3)))
            .expect("symbol response");
        let names: Vec<&str> = symbols["result"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| s["name"].as_str().unwrap())
            .collect();
        assert_eq!(names, vec!["start", "done"]);
        assert_eq!(symbols["result"][0]["range"]["end"]["character"], 12);
    }

    #[test]
    fn hover_definition_and_references_use_lsp_positions() {
        let source = "label start:\n    jump start\n    set broken\n    jump start\n";
        let mut input = stream(&[
            json!({
                "jsonrpc": "2.0",
                "method": "textDocument/didOpen",
                "params": { "textDocument": { "uri": "file:///nav.velin", "text": source } },
            }),
            json!({
                "jsonrpc": "2.0",
                "id": 10,
                "method": "textDocument/hover",
                "params": {
                    "textDocument": { "uri": "file:///nav.velin" },
                    "position": { "line": 1, "character": 9 },
                },
            }),
            json!({
                "jsonrpc": "2.0",
                "id": 11,
                "method": "textDocument/definition",
                "params": {
                    "textDocument": { "uri": "file:///nav.velin" },
                    "position": { "line": 3, "character": 9 },
                },
            }),
            json!({
                "jsonrpc": "2.0",
                "id": 12,
                "method": "textDocument/references",
                "params": {
                    "textDocument": { "uri": "file:///nav.velin" },
                    "position": { "line": 0, "character": 6 },
                    "context": { "includeDeclaration": false },
                },
            }),
            json!({ "jsonrpc": "2.0", "method": "exit" }),
        ]);
        let mut output = Vec::new();
        Server::new(&mut output).run(&mut input).unwrap();
        let messages = parse_all(output);

        let response = |id| {
            messages
                .iter()
                .find(|message| message.get("id") == Some(&json!(id)))
                .unwrap()
        };
        assert!(
            response(10)["result"]["contents"]["value"]
                .as_str()
                .unwrap()
                .contains("defined on line 1")
        );
        assert_eq!(response(11)["result"]["uri"], "file:///nav.velin");
        assert_eq!(response(11)["result"]["range"]["start"]["line"], 0);
        assert_eq!(response(12)["result"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn shutdown_unknown_methods_and_malformed_documents() {
        let mut input = stream(&[
            json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {} }),
            json!({ "jsonrpc": "2.0", "id": 2, "method": "shutdown" }),
            json!({ "jsonrpc": "2.0", "id": 3, "method": "workspace/symbol", "params": {} }),
            json!({ "jsonrpc": "2.0", "method": "initialized" }),
            json!({
                "jsonrpc": "2.0",
                "method": "textDocument/didOpen",
                "params": { "textDocument": { "text": "set x = 1\n" } },
            }),
            json!({
                "jsonrpc": "2.0",
                "method": "textDocument/didChange",
                "params": { "contentChanges": [{ "text": "set x = 2\n" }] },
            }),
            json!({
                "jsonrpc": "2.0",
                "method": "textDocument/didClose",
                "params": { "textDocument": { "uri": "file:///missing.velin" } },
            }),
            json!({
                "jsonrpc": "2.0",
                "id": 4,
                "method": "textDocument/completion",
                "params": { "textDocument": { "uri": "file:///missing.velin" } },
            }),
            json!({
                "jsonrpc": "2.0",
                "id": 5,
                "method": "textDocument/documentSymbol",
                "params": { "textDocument": { "uri": "file:///missing.velin" } },
            }),
            json!({
                "jsonrpc": "2.0",
                "method": "textDocument/didOpen",
                "params": { "textDocument": {
                    "uri": "file:///c.velin",
                    "text": "set x = 1\n",
                } },
            }),
            json!({
                "jsonrpc": "2.0",
                "method": "textDocument/didClose",
                "params": { "textDocument": { "uri": "file:///c.velin" } },
            }),
            json!({ "jsonrpc": "2.0", "method": "exit" }),
        ]);
        let mut output = Vec::new();
        Server::new(&mut output).run(&mut input).unwrap();
        let messages = parse_all(output);
        assert!(messages.iter().any(|m| m.get("id") == Some(&json!(2))));
        assert!(
            messages
                .iter()
                .any(|m| m.get("id") == Some(&json!(3)) && m["error"]["code"] == -32601)
        );
        let completion = messages
            .iter()
            .find(|m| m.get("id") == Some(&json!(4)))
            .expect("completion");
        assert!(
            completion["result"]
                .as_array()
                .unwrap()
                .iter()
                .any(|item| item["label"] == "set")
        );
    }

    #[test]
    fn diagnostics_include_hints_and_warning_severity() {
        let mut diagnostic = Diagnostic::warning("test.velin", 1, 1, "unused");
        diagnostic = diagnostic.with_hint("delete it");
        let converted = lsp_diagnostic(&diagnostic, &["set x = 1"]);
        assert_eq!(converted["severity"], 2);
        assert!(
            converted["message"]
                .as_str()
                .unwrap()
                .contains("hint: delete it")
        );
    }
}
