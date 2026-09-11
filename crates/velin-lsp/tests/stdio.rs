//! Spawns the `velin-lsp` binary so `main` and the stdio transport are covered.

use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};

fn write_message(writer: &mut impl Write, message: &Value) {
    let body = serde_json::to_vec(message).unwrap();
    write!(writer, "Content-Length: {}\r\n\r\n", body.len()).unwrap();
    writer.write_all(&body).unwrap();
    writer.flush().unwrap();
}

#[test]
fn binary_initialize_then_exit() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_velin-lsp"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn velin-lsp");
    {
        let stdin = child.stdin.as_mut().expect("stdin");
        write_message(
            stdin,
            &json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {} }),
        );
        write_message(
            stdin,
            &json!({ "jsonrpc": "2.0", "id": 2, "method": "shutdown" }),
        );
        write_message(stdin, &json!({ "jsonrpc": "2.0", "method": "exit" }));
    }
    let output = child.wait_with_output().expect("wait");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let mut stdout = BufReader::new(output.stdout.as_slice());
    let mut header = String::new();
    stdout.read_line(&mut header).unwrap();
    assert!(header.contains("Content-Length:"));
}
