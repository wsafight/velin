//! Small data-metric and RNG helpers for register evaluation.

use super::Frame;
use velin_eval::EvalError;
use velin_syntax::{DataFootprint, DataMetrics, Value};

pub(super) fn scalar_metrics(_value: &Value) -> DataMetrics {
    DataMetrics {
        footprint: DataFootprint {
            values: 1,
            text_bytes: 0,
        },
        max_depth: 0,
    }
}

pub(super) fn string_metrics(text: &str) -> DataMetrics {
    DataMetrics {
        footprint: DataFootprint {
            values: 1,
            text_bytes: text.len(),
        },
        max_depth: 0,
    }
}

pub(crate) fn shallow_metrics(value: &Value) -> DataMetrics {
    match value {
        Value::String(text) => string_metrics(text),
        _ => scalar_metrics(value),
    }
}

pub(super) fn rng_state(frame: &mut Frame, slot: u32, line: usize) -> Result<&mut i64, EvalError> {
    let value = frame
        .get_mut(slot as usize)
        .and_then(Option::as_mut)
        .ok_or_else(|| EvalError::new(line, "RNG state has not been initialized"))?;
    let Value::Integer(state) = value else {
        return Err(EvalError::new(line, "RNG state must be an integer"));
    };
    Ok(state)
}
