//! Core language types shared across the Velin toolchain.
//!
//! This crate is deliberately free of host-domain concepts. It only knows
//! about values, expressions, built-in functions, source spans, and
//! diagnostics.

pub mod data;
pub mod diagnostic;
pub mod expr;

pub use data::{Builtin, DataFootprint, MAX_DATA_DEPTH, MAX_DATA_TEXT_BYTES, MAX_DATA_VALUES};
pub use diagnostic::{Diagnostic, Severity};
pub use expr::{BinaryOp, Expr, Span, StrPart, UnaryOp, Value};
