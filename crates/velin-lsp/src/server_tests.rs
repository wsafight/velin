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
        json!({ "jsonrpc": "2.0", "id": 99, "method": "shutdown" }),
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
        json!({ "jsonrpc": "2.0", "id": 99, "method": "shutdown" }),
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
        json!({ "jsonrpc": "2.0", "id": 99, "method": "shutdown" }),
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
        json!({ "jsonrpc": "2.0", "id": 99, "method": "shutdown" }),
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
            .any(|m| m.get("id") == Some(&json!(3)) && m["error"]["code"] == -32600)
    );
    let completion = messages
        .iter()
        .find(|m| m.get("id") == Some(&json!(4)))
        .expect("completion error");
    assert_eq!(completion["error"]["code"], -32600);
}

#[test]
fn p3_requests_and_cross_module_navigation_use_workspace_documents() {
    let main = "import math\nset result=call math.add(1,2)\n";
    let module = "export fn add(left, right):\n    return left+right\n";
    let mut input = stream(&[
        json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {} }),
        json!({
            "jsonrpc": "2.0", "method": "textDocument/didOpen",
            "params": { "textDocument": { "uri": "file:///main.velin", "text": main } },
        }),
        json!({
            "jsonrpc": "2.0", "method": "textDocument/didOpen",
            "params": { "textDocument": { "uri": "file:///math.velin", "text": module } },
        }),
        json!({
            "jsonrpc": "2.0", "id": 30, "method": "textDocument/definition",
            "params": { "textDocument": { "uri": "file:///main.velin" }, "position": { "line": 1, "character": 22 } },
        }),
        json!({
            "jsonrpc": "2.0", "id": 31, "method": "textDocument/references",
            "params": { "textDocument": { "uri": "file:///main.velin" }, "position": { "line": 1, "character": 22 }, "context": { "includeDeclaration": true } },
        }),
        json!({
            "jsonrpc": "2.0", "id": 32, "method": "textDocument/rename",
            "params": { "textDocument": { "uri": "file:///main.velin" }, "position": { "line": 1, "character": 22 }, "newName": "sum" },
        }),
        json!({
            "jsonrpc": "2.0", "id": 33, "method": "textDocument/formatting",
            "params": { "textDocument": { "uri": "file:///main.velin" }, "options": {} },
        }),
        json!({
            "jsonrpc": "2.0", "id": 34, "method": "textDocument/semanticTokens/full",
            "params": { "textDocument": { "uri": "file:///main.velin" } },
        }),
        json!({
            "jsonrpc": "2.0", "id": 35, "method": "textDocument/codeAction",
            "params": { "textDocument": { "uri": "file:///main.velin" }, "range": {}, "context": { "diagnostics": [] } },
        }),
        json!({ "jsonrpc": "2.0", "id": 36, "method": "workspace/symbol", "params": { "query": "add" } }),
        json!({ "jsonrpc": "2.0", "id": 99, "method": "shutdown" }),
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

    assert_eq!(response(30)["result"]["uri"], "file:///math.velin");
    assert_eq!(response(31)["result"].as_array().unwrap().len(), 2);
    assert_eq!(
        response(32)["result"]["changes"]["file:///math.velin"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(response(33)["result"].as_array().unwrap().len(), 1);
    assert!(
        !response(34)["result"]["data"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert_eq!(response(35)["result"].as_array().unwrap().len(), 1);
    assert!(response(35)["result"][0]["edit"]["changes"]["file:///main.velin"].is_array());
    assert!(
        response(36)["result"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| {
                item["name"] == "add" && item["location"]["uri"] == "file:///math.velin"
            })
    );
    let latest_main_diagnostics = messages
        .iter()
        .rev()
        .find(|message| {
            message["method"] == "textDocument/publishDiagnostics"
                && message["params"]["uri"] == "file:///main.velin"
        })
        .unwrap();
    assert!(
        latest_main_diagnostics["params"]["diagnostics"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}

#[test]
fn module_dependency_diagnostics_report_missing_and_cycles_at_imports() {
    let mut documents = HashMap::new();
    documents.insert("file:///a.velin".to_owned(), "import b\n".to_owned());
    let missing = module_diagnostics("file:///a.velin", "import b\n", &documents);
    assert!(missing[0].message.contains("cannot resolve module `b`"));

    documents.insert("file:///b.velin".to_owned(), "import a\n".to_owned());
    let cycle = module_diagnostics("file:///a.velin", "import b\n", &documents);
    assert!(cycle[0].message.contains("cyclic module import"));
    assert_eq!(cycle[0].line, 1);
    assert_eq!(cycle[0].column, 8);
}

#[test]
fn exit_without_shutdown_is_a_process_error() {
    let mut input = stream(&[
        json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {} }),
        json!({ "jsonrpc": "2.0", "method": "exit" }),
    ]);
    let mut output = Vec::new();
    let error = Server::new(&mut output).run(&mut input).unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::Other);
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

#[test]
fn configured_schema_reaches_every_host_editor_feature() {
    let schema = HostSchema::new().declare(
        velin::HostCommand::new(
            "ask",
            velin::HostSignature::exact(vec![velin::Type::String], Some(velin::Type::Integer)),
        )
        .description("Ask the player."),
    );
    let source = "answer = perform ask(\"Ready?\")\n";
    let mut input = stream(&[
        json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didOpen",
            "params": { "textDocument": {
                "uri": "file:///host.velin",
                "text": source,
            } },
        }),
        json!({
            "jsonrpc": "2.0",
            "id": 20,
            "method": "textDocument/completion",
            "params": { "textDocument": { "uri": "file:///host.velin" } },
        }),
        json!({
            "jsonrpc": "2.0",
            "id": 21,
            "method": "textDocument/hover",
            "params": {
                "textDocument": { "uri": "file:///host.velin" },
                "position": { "line": 0, "character": 19 },
            },
        }),
        json!({
            "jsonrpc": "2.0",
            "id": 22,
            "method": "textDocument/signatureHelp",
            "params": {
                "textDocument": { "uri": "file:///host.velin" },
                "position": { "line": 0, "character": 28 },
            },
        }),
        json!({ "jsonrpc": "2.0", "id": 99, "method": "shutdown" }),
        json!({ "jsonrpc": "2.0", "method": "exit" }),
    ]);
    let mut output = Vec::new();
    Server::with_host_schema(&mut output, schema)
        .run(&mut input)
        .unwrap();
    let messages = parse_all(output);
    let response = |id| {
        messages
            .iter()
            .find(|message| message.get("id") == Some(&json!(id)))
            .unwrap()
    };
    assert!(
        response(20)["result"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["label"] == "ask" && item["documentation"] == "Ask the player.")
    );
    assert!(
        response(21)["result"]["contents"]["value"]
            .as_str()
            .unwrap()
            .contains("Ask the player.")
    );
    assert_eq!(
        response(22)["result"]["signatures"][0]["label"],
        "ask(string) -> integer"
    );
}
