use super::*;
use serde_json::json;
use std::io::Cursor;

#[test]
fn round_trips_a_message() {
    let message = json!({ "jsonrpc": "2.0", "id": 1, "method": "ping" });
    let mut buffer = Vec::new();
    write_message(&mut buffer, &message).unwrap();

    let framed = String::from_utf8(buffer.clone()).unwrap();
    assert!(framed.starts_with("Content-Length: "));
    assert!(framed.contains("\r\n\r\n"));

    let mut cursor = Cursor::new(buffer);
    let read = read_message(&mut cursor).unwrap().unwrap();
    assert_eq!(read, message);
    // A second read hits EOF.
    assert!(read_message(&mut cursor).unwrap().is_none());
}

#[test]
fn eof_before_any_header_is_none() {
    let mut cursor = Cursor::new(Vec::new());
    assert!(read_message(&mut cursor).unwrap().is_none());
}

#[test]
fn missing_content_length_is_invalid() {
    let mut cursor = Cursor::new(b"Content-Type: application/vscode-jsonrpc\r\n\r\n".to_vec());
    let error = read_message(&mut cursor).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
}

#[test]
fn truncated_headers_and_bodies_are_errors() {
    let cases = [
        b"Content-Length: 1\r\n".as_slice(),
        b"Content-Length: 1\r\n\r\n".as_slice(),
        b"Content-Length: 1\r\nContent-Type: application/json\r\n".as_slice(),
    ];
    for input in cases {
        let error = read_message(&mut Cursor::new(input)).unwrap_err();
        assert!(matches!(
            error.kind(),
            io::ErrorKind::UnexpectedEof | io::ErrorKind::InvalidData
        ));
    }
}

#[test]
fn oversized_and_malformed_lengths_are_rejected_before_allocation() {
    let oversized = format!("Content-Length: {}\r\n\r\n", MAX_LSP_MESSAGE_BYTES + 1);
    let error = read_message(&mut Cursor::new(oversized)).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::InvalidData);

    let malformed = b"Content-Length: nope\r\n\r\n".to_vec();
    let error = read_message(&mut Cursor::new(malformed)).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
}
