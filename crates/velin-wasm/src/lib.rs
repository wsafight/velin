//! WebAssembly bindings for Velin — the browser Playground's engine.
//!
//! The whole point of Velin on the web is "the wasm thinks, JS performs": this
//! crate marshals a `.velin` **source string** in and a **JSON result** out,
//! and never touches the DOM. Two entry points mirror the CLI:
//!
//! * [`check`] — compile + static-check, returning the diagnostics.
//! * [`run`] — check, then drive a *scripted* reference host (`say` appends to
//!   an output log; `ask` is answered from a caller-supplied list of replies)
//!   so a run is fully deterministic and needs no interactive stdin.
//!
//! The language logic all lives in `velin`; this file is pure marshalling, so
//! the core's determinism / no-float / no-`unsafe` guarantees are preserved
//! (the only `unsafe` in the built artifact is wasm-bindgen's generated glue).
//!
//! Everything is implemented on plain Rust types in the private engine module and unit-tested
//! natively; the `#[wasm_bindgen]` layer is a thin string wrapper on top.

#[cfg(feature = "source")]
use serde::Serialize;
use wasm_bindgen::prelude::wasm_bindgen;

#[cfg(feature = "source")]
mod engine;
#[cfg(feature = "runtime")]
mod runtime;

/// Checks a `.velin` `source` and returns a JSON `CheckResult`:
/// `{ "ok": bool, "diagnostics": [ { severity, line, column, message } ] }`.
#[cfg(feature = "source")]
#[wasm_bindgen]
#[must_use]
pub fn check(source: &str) -> String {
    to_json(&engine::check("playground.velin", source))
}

/// Runs a `.velin` `source` against the scripted host and returns a JSON
/// `RunResult`. `replies_json` is a JSON array of answers for `ask` effects,
/// consumed in order (integers, `true`/`false`, or strings); pass `"[]"` for
/// none. Malformed JSON or unsupported values return a failed `RunResult`
/// without executing the script.
#[cfg(feature = "source")]
#[wasm_bindgen]
#[must_use]
pub fn run(source: &str, replies_json: &str) -> String {
    let result = match engine::parse_replies(replies_json) {
        Ok(replies) => engine::run("playground.velin", source, replies),
        Err(error) => engine::RunResult {
            ok: false,
            diagnostics: Vec::new(),
            output: Vec::new(),
            error: Some(error.to_string()),
        },
    };
    to_json(&result)
}

/// A persistent runtime-only machine for hosts that already have bytecode.
#[cfg(feature = "runtime")]
#[wasm_bindgen]
pub struct RuntimeMachine {
    machine: velin_vm::Machine,
}

#[cfg(feature = "runtime")]
#[wasm_bindgen]
impl RuntimeMachine {
    /// Loads and validates a JSON-encoded [`velin_bytecode::Program`].
    ///
    /// # Errors
    ///
    /// Returns a JavaScript error when the JSON is malformed or the program
    /// fails bytecode validation.
    #[wasm_bindgen(constructor)]
    pub fn new(program_json: &str) -> Result<RuntimeMachine, wasm_bindgen::JsValue> {
        let program: velin_bytecode::Program =
            serde_json::from_str(program_json).map_err(|error| runtime::json_error(&error))?;
        let machine =
            velin_vm::Machine::new(program).map_err(|error| runtime::runtime_error(&error))?;
        Ok(Self { machine })
    }

    /// Runs until a host event, completion, or a bounded execution error.
    #[must_use]
    pub fn run(&mut self) -> String {
        runtime::yield_to_json(self.machine.run())
    }

    /// Collects side-effect-only host events in source order until `limit`, a
    /// bound host event, completion, or an execution error.
    #[must_use]
    pub fn run_batch(&mut self, limit: usize) -> String {
        runtime::batch_to_json(self.machine.run_effect_batch(limit))
    }

    /// Resumes a pending host event. Use the JSON literal `null` when the host
    /// command has no return value.
    #[must_use]
    pub fn resume(&mut self, value_json: &str) -> String {
        let value = match serde_json::from_str(value_json) {
            Ok(value) => value,
            Err(error) => return runtime::error_to_json(format!("invalid resume value: {error}")),
        };
        runtime::yield_to_json(self.machine.resume(value))
    }
}

/// Serializes a result to JSON, falling back to a minimal error object so the
/// browser always receives valid JSON.
#[cfg(feature = "source")]
fn to_json<T: Serialize>(value: &T) -> String {
    serde_json::to_string(value)
        .unwrap_or_else(|_| "{\"ok\":false,\"diagnostics\":[],\"output\":[]}".to_owned())
}

#[cfg(test)]
#[cfg(feature = "source")]
#[path = "lib_tests.rs"]
mod tests;
