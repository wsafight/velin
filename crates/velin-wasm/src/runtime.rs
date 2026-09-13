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

pub fn runtime_error(error: &velin_bytecode::ProgramValidationError) -> wasm_bindgen::JsValue {
    wasm_bindgen::JsValue::from_str(&format!("invalid program: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use velin_bytecode::{ExprChunk, ExprOp, Op, Program, SlotTable};

    fn program_json() -> String {
        let mut slots = SlotTable::new();
        let result = slots.intern("result");
        let mut chunk = ExprChunk::new(1);
        let constant = chunk.constant(Value::Integer(42));
        chunk.push(ExprOp::Const {
            dst: chunk.result,
            constant,
        });
        let program = Program::from_chunks(
            vec![
                Op::Set {
                    slot: result,
                    value: 0,
                },
                Op::Halt,
            ],
            vec![chunk],
            slots,
        );
        serde_json::to_string(&program).unwrap()
    }

    #[test]
    fn runtime_machine_executes_a_program_without_the_source_frontend() {
        let program: Program = serde_json::from_str(&program_json()).unwrap();
        let mut machine = velin_vm::Machine::new(program).unwrap();
        assert_eq!(machine.run(), Ok(Yield::Finished));
    }

    #[test]
    fn results_have_a_small_tagged_wire_shape() {
        let json = yield_to_json(Ok::<_, String>(Yield::Finished));
        assert_eq!(json, "{\"kind\":\"finished\"}");
    }

    #[test]
    fn batch_results_preserve_host_order() {
        let effects = vec![
            HostEffect {
                host_id: 2,
                values: vec![Value::Integer(1)],
            },
            HostEffect {
                host_id: 3,
                values: Vec::new(),
            },
        ];
        let json = batch_to_json(Ok::<_, String>(effects));
        assert!(json.contains("\"host_id\":2"));
        assert!(json.contains("\"host_id\":3"));
    }
}
