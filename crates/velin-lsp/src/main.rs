//! `velin-lsp` — a minimal Language Server for the Velin statement language.
//!
//! It speaks LSP over stdio and offers editor capabilities as thin adaptors
//! over the `velin` library:
//!
//! * **diagnostics** — pushed on open/change; a compile error, or the
//!   static-check findings (`velin-check`), rendered as LSP diagnostics.
//! * **completion** — the fixed keyword/builtin vocabulary plus the variables
//!   and labels the current document declares.
//! * **document symbols** — the script's labels, for the outline view.
//! * **hover and navigation** — language help plus label definitions/references.
//!
//! The wire framing lives in [`protocol`], the language analysis in
//! [`analysis`], and the request loop in [`server`]. Point any LSP client at
//! the `velin-lsp` binary (stdio transport) to use it.

mod analysis;
mod protocol;
mod server;

use server::Server;
use std::io::{self, BufReader};

fn main() -> io::Result<()> {
    let stdin = io::stdin();
    let mut input = BufReader::new(stdin.lock());
    let stdout = io::stdout();
    let output = stdout.lock();
    Server::new(output).run(&mut input)
}
