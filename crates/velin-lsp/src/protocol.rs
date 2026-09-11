//! JSON-RPC over stdio: the `Content-Length`-framed message transport every
//! Language Server Protocol client speaks.
//!
//! This is deliberately the whole wire layer — a header block terminated by a
//! blank line, then exactly `Content-Length` bytes of JSON — so the rest of the
//! server only ever sees [`serde_json::Value`]s and never touches framing.

use serde_json::Value;
use std::io::{self, BufRead, Read, Write};

/// Maximum JSON body accepted from an editor client.
pub const MAX_LSP_MESSAGE_BYTES: usize = 4 * 1024 * 1024;
const MAX_LSP_HEADER_BYTES: usize = 64 * 1024;
const MAX_LSP_HEADER_LINE_BYTES: usize = 8 * 1024;

/// Reads one framed message. Returns `Ok(None)` at end of input (the client
/// closed the pipe) so the caller's loop can exit cleanly.
///
/// # Errors
/// Returns an [`io::Error`] if the stream fails or the body is not valid JSON.
pub fn read_message(reader: &mut impl BufRead) -> io::Result<Option<Value>> {
    let mut content_length: Option<usize> = None;
    let mut header_bytes = 0usize;
    loop {
        let mut line = String::new();
        let read = reader
            .take((MAX_LSP_HEADER_LINE_BYTES + 1) as u64)
            .read_line(&mut line)?;
        if read == 0 {
            return Ok(None); // EOF between messages: a clean shutdown.
        }
        if read > MAX_LSP_HEADER_LINE_BYTES {
            return Err(invalid_data("LSP header line exceeds 8 KiB"));
        }
        header_bytes = header_bytes
            .checked_add(read)
            .filter(|bytes| *bytes <= MAX_LSP_HEADER_BYTES)
            .ok_or_else(|| invalid_data("LSP headers exceed 64 KiB"))?;
        let trimmed = line.trim_end();
        if trimmed.is_empty() {
            break; // Blank line: headers are done, the body follows.
        }
        if let Some(rest) = trimmed.strip_prefix("Content-Length:") {
            if content_length.is_some() {
                return Err(invalid_data("duplicate Content-Length header"));
            }
            content_length = Some(
                rest.trim()
                    .parse()
                    .map_err(|_| invalid_data("invalid Content-Length header"))?,
            );
        }
    }

    let Some(length) = content_length else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "message header had no Content-Length",
        ));
    };
    if length > MAX_LSP_MESSAGE_BYTES {
        return Err(invalid_data("LSP message exceeds 4 MiB"));
    }

    let mut body = vec![0u8; length];
    reader.read_exact(&mut body)?;
    let value = serde_json::from_slice(&body).map_err(io::Error::other)?;
    Ok(Some(value))
}

fn invalid_data(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

/// Writes one framed message and flushes it.
///
/// # Errors
/// Returns an [`io::Error`] if serialization or the underlying write fails.
pub fn write_message(writer: &mut impl Write, message: &Value) -> io::Result<()> {
    let body = serde_json::to_vec(message).map_err(io::Error::other)?;
    write!(writer, "Content-Length: {}\r\n\r\n", body.len())?;
    writer.write_all(&body)?;
    writer.flush()
}

#[cfg(test)]
mod tests {
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
    fn oversized_and_malformed_lengths_are_rejected_before_allocation() {
        let oversized = format!("Content-Length: {}\r\n\r\n", MAX_LSP_MESSAGE_BYTES + 1);
        let error = read_message(&mut Cursor::new(oversized)).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);

        let malformed = b"Content-Length: nope\r\n\r\n".to_vec();
        let error = read_message(&mut Cursor::new(malformed)).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    }
}
