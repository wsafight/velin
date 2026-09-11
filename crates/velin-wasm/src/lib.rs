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
//! Everything is implemented on plain Rust types in [`engine`] and unit-tested
//! natively; the `#[wasm_bindgen]` layer is a thin string wrapper on top.

use serde::Serialize;
use wasm_bindgen::prelude::wasm_bindgen;

mod engine;

/// Checks a `.velin` `source` and returns a JSON `CheckResult`:
/// `{ "ok": bool, "diagnostics": [ { severity, line, column, message } ] }`.
#[wasm_bindgen]
#[must_use]
pub fn check(source: &str) -> String {
    to_json(&engine::check("playground.velin", source))
}

/// Runs a `.velin` `source` against the scripted host and returns a JSON
/// `RunResult`. `replies_json` is a JSON array of answers for `ask` effects,
/// consumed in order (integers, `true`/`false`, or strings); pass `"[]"` for
/// none. Malformed JSON is treated as an empty reply list.
#[wasm_bindgen]
#[must_use]
pub fn run(source: &str, replies_json: &str) -> String {
    let replies = engine::parse_replies(replies_json);
    to_json(&engine::run("playground.velin", source, replies))
}

/// Serializes a result to JSON, falling back to a minimal error object so the
/// browser always receives valid JSON.
fn to_json<T: Serialize>(value: &T) -> String {
    serde_json::to_string(value)
        .unwrap_or_else(|_| "{\"ok\":false,\"diagnostics\":[],\"output\":[]}".to_owned())
}

#[cfg(test)]
mod tests {
    #[test]
    fn wasm_bindings_return_json() {
        let checked = super::check("set x = 1\n");
        assert!(checked.contains("\"ok\":true"), "{checked}");
        let ran = super::run("perform say(\"hi\")\n", "[]");
        assert!(ran.contains("hi"), "{ran}");
        let replies = super::run(
            "choice = perform ask(\"go?\")\nperform say(choice)\n",
            "[1]",
        );
        assert!(replies.contains('1'), "{replies}");
        let garbage = super::run("perform say(\"ok\")\n", "not-json");
        assert!(garbage.contains("ok"), "{garbage}");
    }
}
