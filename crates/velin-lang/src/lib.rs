//! Velin language front-end.
//!
//! The core crates (`velin-syntax`, `velin-parse`, `velin-compile`,
//! `velin-vm`, `velin-check`) give a full *expression* pipeline: a host builds
//! a [`velin_compile::Program`] by hand with `ProgramBuilder`. That makes velin
//! an embeddable evaluation *library*, but there is no surface syntax — nobody
//! can write a `.velin` file.
//!
//! `velin-lang` adds exactly that missing upper half: an indentation-sensitive
//! *statement* language that lowers to the existing bytecode `Program`. It
//! reuses [`velin_parse::parse_expression`] for every expression and condition,
//! so it introduces no new value semantics or host-domain concepts. A call
//! like `emit("hi")` is not a keyword but a **host command**, interned to an
//! opaque `host_id` and lowered to [`velin_compile::Op::Host`].
//!
//! ```text
//! source ──parse──▶ Ast ──lower──▶ CompiledScript { program, hosts, labels, defaults }
//!                    │                                   │
//!                  (check)                          Machine::new(program)
//! ```
//!
//! # Example
//!
//! ```
//! use velin_lang::compile;
//!
//! let script = compile(
//!     "heal.velin",
//!     "default hp = 30\n\
//!      set hp = hp + 10\n\
//!      if hp > 20:\n\
//!      \x20\x20\x20\x20perform say(\"healthy\")\n",
//! )
//! .unwrap();
//! assert_eq!(script.defaults.get("hp"), Some(&velin_syntax::Value::Integer(30)));
//! assert_eq!(script.hosts, vec!["say".to_string()]);
//! ```

mod ast;
mod error;
mod host;
mod limits;
mod lines;
mod lower;
mod parser;
mod token;

pub use ast::{Condition, Stmt};
pub use error::ParseError;
pub use host::CompiledScript;
pub use limits::{MAX_SOURCE_BYTES, MAX_SOURCE_LINES, MAX_STATEMENT_DEPTH};
pub use lower::LowerError;

use velin_syntax::Diagnostic;

/// The file name attached to a diagnostic when the caller does not supply one
/// (for example [`parse_program`], which returns unrendered [`ParseError`]s).
/// The `compile`/`check_script` entry points take a real `file` argument and
/// thread it all the way down, so operator-facing diagnostics point at the
/// script's actual path instead of this placeholder.
pub(crate) const FILE: &str = "<velin>";

/// Parses `source` into a statement AST without lowering it.
///
/// # Errors
/// Returns a [`ParseError`] (with a source location) for malformed syntax:
/// bad indentation, an unknown statement keyword, or an invalid embedded
/// expression. A `ParseError` carries a line/column but no file name; render
/// it with [`ParseError::into_diagnostic`], passing the source path.
pub fn parse_program(source: &str) -> Result<Vec<Stmt>, ParseError> {
    parser::parse(source)
}

/// Parses and lowers `source` (named `file` for diagnostics) into a runnable
/// [`CompiledScript`].
///
/// # Errors
/// Returns a [`Diagnostic`] for any parse error or lowering error (e.g. a
/// `jump` to an undefined label). The diagnostic's `file` field is `file`.
pub fn compile(file: &str, source: &str) -> Result<CompiledScript, Diagnostic> {
    let statements = parse_program(source).map_err(|error| error.into_diagnostic(file))?;
    lower::lower(&statements).map_err(|error| error.into_diagnostic(file))
}

/// Runs Velin's conservative static checks over a compiled script: definite
/// assignment (every read is assigned on all paths, with `default`s seeded) and
/// expression type inference. Diagnostics are attributed to `file`.
///
/// Returns structured diagnostics; an empty vector means the checks found
/// nothing provably wrong.
#[must_use]
pub fn check_script(file: &str, script: &CompiledScript) -> Vec<Diagnostic> {
    script.check(file)
}
