use super::Server;
use crate::protocol::{read_message, write_message};
use serde_json::{Value, json};
use std::io::Cursor;

fn stream(messages: &[Value]) -> Cursor<Vec<u8>> {
    let mut buffer = Vec::new();
    for message in messages {
        write_message(&mut buffer, message).unwrap();
    }
    Cursor::new(buffer)
}

fn parse_all(bytes: Vec<u8>) -> Vec<Value> {
    let mut cursor = Cursor::new(bytes);
    let mut output = Vec::new();
    while let Some(message) = read_message(&mut cursor).unwrap() {
        output.push(message);
    }
    output
}

#[test]
fn initialize_indexes_unopened_workspace_modules() {
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "velin-lsp-workspace-{}-{unique}",
        std::process::id()
    ));
    std::fs::create_dir(&root).unwrap();
    std::fs::write(
        root.join("main.velin"),
        "import math\nset result = call math.add(1, 2)\n",
    )
    .unwrap();
    std::fs::write(
        root.join("math.velin"),
        "export fn add(left, right):\n    return left + right\n",
    )
    .unwrap();
    let root_uri = format!("file://{}", root.display());
    let main_uri = format!("{root_uri}/main.velin");
    let math_uri = format!("{root_uri}/math.velin");
    let mut input = stream(&[
        json!({
            "jsonrpc": "2.0", "id": 1, "method": "initialize",
            "params": { "rootUri": root_uri },
        }),
        json!({
            "jsonrpc": "2.0", "id": 2, "method": "textDocument/definition",
            "params": {
                "textDocument": { "uri": main_uri },
                "position": { "line": 1, "character": 24 },
            },
        }),
        json!({
            "jsonrpc": "2.0", "id": 3, "method": "workspace/symbol",
            "params": { "query": "add" },
        }),
        json!({ "jsonrpc": "2.0", "id": 99, "method": "shutdown" }),
        json!({ "jsonrpc": "2.0", "method": "exit" }),
    ]);
    let mut output = Vec::new();
    Server::new(&mut output).run(&mut input).unwrap();
    let messages = parse_all(output);
    std::fs::remove_dir_all(root).unwrap();

    let response = |id| {
        messages
            .iter()
            .find(|message| message.get("id") == Some(&json!(id)))
            .unwrap()
    };
    assert_eq!(response(2)["result"]["uri"], math_uri);
    assert!(
        response(3)["result"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| { item["name"] == "add" && item["location"]["uri"] == math_uri })
    );
}
