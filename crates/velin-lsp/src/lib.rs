//! Reusable Velin language analysis and LSP server.

pub mod analysis;
mod protocol;
mod server;

pub use server::Server;
