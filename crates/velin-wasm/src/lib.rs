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
#[cfg(any(feature = "source", feature = "runtime"))]
mod policy;
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
/// consumed in order. Integers, booleans, strings, lists, and records are
/// accepted; pass `"[]"` for none. Malformed, unsupported, or over-budget
/// values return a failed `RunResult` without executing the script.
#[cfg(feature = "source")]
#[wasm_bindgen]
#[must_use]
pub fn run(source: &str, replies_json: &str) -> String {
    run_with_policy(source, replies_json, "{}")
}

/// Runs source with a JSON-encoded numeric execution policy.
#[cfg(feature = "source")]
#[wasm_bindgen]
#[must_use]
pub fn run_with_policy(source: &str, replies_json: &str, policy_json: &str) -> String {
    let result = match engine::parse_replies(replies_json) {
        Ok(replies) => match policy::parse(policy_json) {
            Ok(policy) => engine::run_with_policy("playground.velin", source, replies, policy),
            Err(error) => engine::RunResult {
                ok: false,
                diagnostics: Vec::new(),
                output: Vec::new(),
                error: Some(error),
            },
        },
        Err(error) => engine::RunResult {
            ok: false,
            diagnostics: Vec::new(),
            output: Vec::new(),
            error: Some(error.to_string()),
        },
    };
    to_json(&result)
}

/// Persistent Playground debugger with pause, resume, stepping, snapshots and replay.
#[cfg(feature = "source")]
#[wasm_bindgen]
pub struct PlaygroundSession {
    session: Option<engine::InteractiveSession>,
    initial: engine::DebugResult,
}

#[cfg(feature = "source")]
#[wasm_bindgen]
impl PlaygroundSession {
    #[wasm_bindgen(constructor)]
    #[must_use]
    pub fn new(source: &str) -> PlaygroundSession {
        Self::new_with_policy(source, "{}")
    }

    /// Creates a Playground debugger with a JSON-encoded numeric policy.
    #[must_use]
    pub fn new_with_policy(source: &str, policy_json: &str) -> PlaygroundSession {
        let policy = match policy::parse(policy_json) {
            Ok(policy) => policy,
            Err(error) => {
                return Self::from_error(engine::DebugResult {
                    ok: false,
                    status: "error".to_owned(),
                    line: 0,
                    diagnostics: Vec::new(),
                    output: Vec::new(),
                    variables: std::collections::BTreeMap::new(),
                    snapshot: None,
                    error: Some(error),
                });
            }
        };
        match engine::InteractiveSession::new_with_policy("playground.velin", source, policy) {
            Ok(session) => {
                let initial = session.state();
                Self {
                    session: Some(session),
                    initial,
                }
            }
            Err(initial) => Self {
                session: None,
                initial: *initial,
            },
        }
    }

    fn from_error(initial: engine::DebugResult) -> PlaygroundSession {
        Self {
            session: None,
            initial,
        }
    }

    #[must_use]
    pub fn state(&self) -> String {
        self.session.as_ref().map_or_else(
            || to_json(&self.initial),
            |session| to_json(&session.state()),
        )
    }

    #[must_use]
    pub fn resume(&mut self, replies_json: &str) -> String {
        self.with_replies(replies_json, false)
    }

    #[must_use]
    pub fn step(&mut self, replies_json: &str) -> String {
        self.with_replies(replies_json, true)
    }

    #[must_use]
    pub fn snapshot(&mut self) -> String {
        self.session.as_mut().map_or_else(
            || to_json(&self.initial),
            |session| to_json(&session.snapshot()),
        )
    }

    #[must_use]
    pub fn restore(&mut self, snapshot: u32) -> String {
        self.session.as_mut().map_or_else(
            || to_json(&self.initial),
            |session| to_json(&session.restore(snapshot)),
        )
    }

    /// Requests cooperative cancellation at the next VM checkpoint.
    pub fn cancel(&mut self) -> String {
        self.session.as_mut().map_or_else(
            || to_json(&self.initial),
            |session| to_json(&session.cancel()),
        )
    }

    /// Clears a previous cooperative cancellation request.
    pub fn clear_cancellation(&mut self) -> String {
        self.session.as_mut().map_or_else(
            || to_json(&self.initial),
            |session| to_json(&session.clear_cancellation()),
        )
    }

    fn with_replies(&mut self, replies_json: &str, step: bool) -> String {
        let replies = match engine::parse_replies(replies_json) {
            Ok(replies) => replies,
            Err(error) => {
                return to_json(&engine::DebugResult {
                    ok: false,
                    status: "error".to_owned(),
                    line: 0,
                    diagnostics: Vec::new(),
                    output: Vec::new(),
                    variables: std::collections::BTreeMap::new(),
                    snapshot: None,
                    error: Some(error.to_string()),
                });
            }
        };
        self.session.as_mut().map_or_else(
            || to_json(&self.initial),
            |session| {
                if step {
                    to_json(&session.step(replies))
                } else {
                    to_json(&session.continue_execution(replies))
                }
            },
        )
    }
}

/// A persistent runtime-only machine for hosts that already have bytecode.
#[cfg(feature = "runtime")]
#[wasm_bindgen]
pub struct RuntimeMachine {
    machine: velin_vm::Machine,
}

#[cfg(feature = "runtime")]
const MAX_RUNTIME_VALUE_JSON_BYTES: usize = 1024 * 1024;

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
        Self::new_with_policy(program_json, "{}")
    }

    /// Loads a program with a JSON-encoded numeric execution policy.
    ///
    /// # Errors
    /// Returns a JavaScript error when the program, policy JSON, or bytecode is invalid.
    pub fn new_with_policy(
        program_json: &str,
        policy_json: &str,
    ) -> Result<RuntimeMachine, wasm_bindgen::JsValue> {
        let program: velin_bytecode::Program =
            serde_json::from_str(program_json).map_err(|error| runtime::json_error(&error))?;
        let policy =
            policy::parse(policy_json).map_err(|error| wasm_bindgen::JsValue::from_str(&error))?;
        let machine = velin_vm::Machine::new_with_policy(program, policy)
            .map_err(|error| runtime::runtime_error(&error))?;
        Ok(Self { machine })
    }

    /// Requests cooperative cancellation at the next VM checkpoint.
    pub fn cancel(&mut self) {
        self.machine.cancel();
    }

    /// Clears a previous cooperative cancellation request.
    pub fn clear_cancellation(&mut self) {
        self.machine.clear_cancellation();
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
        if value_json.len() > MAX_RUNTIME_VALUE_JSON_BYTES {
            return runtime::error_to_json("invalid resume value: JSON exceeds 1 MiB");
        }
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
