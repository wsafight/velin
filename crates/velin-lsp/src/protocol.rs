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
            if header_bytes == 0 {
                return Ok(None); // EOF between messages: a clean shutdown.
            }
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "truncated LSP message header",
            ));
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
#[path = "protocol_tests.rs"]
mod tests;
