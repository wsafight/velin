//! Runtime-only Wasm boundary for prevalidated program data.

use serde::Serialize;
use velin_syntax::Value;
use velin_vm::{HostEffect, Yield};

#[derive(Debug, Serialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum RuntimeYield {
    Host { host_id: u32, values: Vec<Value> },
    Finished,
    Error { message: String },
}

#[derive(Debug, Serialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum RuntimeBatch {
    Effects { effects: Vec<RuntimeEffect> },
    Empty,
    Error { message: String },
}

#[derive(Debug, Serialize, PartialEq, Eq)]
struct RuntimeEffect {
    host_id: u32,
    values: Vec<Value>,
}

pub fn yield_to_json<E: std::fmt::Display>(result: Result<Yield, E>) -> String {
    let output = match result {
        Ok(Yield::Host { host_id, values }) => RuntimeYield::Host { host_id, values },
        Ok(Yield::Finished) => RuntimeYield::Finished,
        Err(error) => RuntimeYield::Error {
            message: error.to_string(),
        },
    };
    serde_json::to_string(&output).unwrap_or_else(|_| error_to_json("cannot encode runtime result"))
}

pub fn error_to_json(message: impl Into<String>) -> String {
    serde_json::to_string(&RuntimeYield::Error {
        message: message.into(),
    })
    .unwrap_or_else(|_| "{\"kind\":\"error\",\"message\":\"runtime error\"}".to_owned())
}

pub fn batch_to_json<E: std::fmt::Display>(result: Result<Vec<HostEffect>, E>) -> String {
    let output = match result {
        Ok(effects) if effects.is_empty() => RuntimeBatch::Empty,
        Ok(effects) => RuntimeBatch::Effects {
            effects: effects
                .into_iter()
                .map(|effect| RuntimeEffect {
                    host_id: effect.host_id,
                    values: effect.values,
                })
                .collect(),
        },
        Err(error) => RuntimeBatch::Error {
            message: error.to_string(),
        },
    };
    serde_json::to_string(&output).unwrap_or_else(|_| error_to_json("cannot encode runtime result"))
}

pub fn json_error(error: &serde_json::Error) -> wasm_bindgen::JsValue {
    wasm_bindgen::JsValue::from_str(&format!("invalid program JSON: {error}"))
}

#[cfg(test)]
#[path = "runtime_tests.rs"]
mod tests;
