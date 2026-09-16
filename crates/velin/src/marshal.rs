//! Budget-aware conversion between Serde data and Velin values.

use crate::{MAX_DATA_DEPTH, MAX_DATA_TEXT_BYTES, MAX_DATA_VALUES, Value};
use serde::Serialize;
use serde::de::{DeserializeOwned, IntoDeserializer};
use serde_json::Value as JsonValue;
use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

/// Resource limits applied while constructing either representation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MarshallingLimits {
    pub max_values: usize,
    pub max_depth: usize,
    pub max_text_bytes: usize,
}

impl Default for MarshallingLimits {
    fn default() -> Self {
        Self {
            max_values: MAX_DATA_VALUES,
            max_depth: MAX_DATA_DEPTH,
            max_text_bytes: MAX_DATA_TEXT_BYTES,
        }
    }
}

/// A conversion failure with a JSON-style path to the offending value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarshalError {
    path: String,
    message: String,
}

impl MarshalError {
    fn new(path: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            message: message.into(),
        }
    }

    #[must_use]
    pub fn path(&self) -> &str {
        &self.path
    }

    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for MarshalError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.path, self.message)
    }
}

impl std::error::Error for MarshalError {}

#[derive(Debug)]
struct Budget {
    limits: MarshallingLimits,
    values: usize,
    text_bytes: usize,
}

impl Budget {
    fn new(limits: MarshallingLimits) -> Self {
        Self {
            limits,
            values: 0,
            text_bytes: 0,
        }
    }

    fn value(&mut self, path: &str, depth: usize) -> Result<(), MarshalError> {
        if depth > self.limits.max_depth {
            return Err(MarshalError::new(
                path,
                format!("nesting exceeds {} levels", self.limits.max_depth),
            ));
        }
        self.values = self
            .values
            .checked_add(1)
            .ok_or_else(|| MarshalError::new(path, "value count overflow"))?;
        if self.values > self.limits.max_values {
            return Err(MarshalError::new(
                path,
                format!("value count exceeds {}", self.limits.max_values),
            ));
        }
        Ok(())
    }

    fn text(&mut self, path: &str, bytes: usize) -> Result<(), MarshalError> {
        self.text_bytes = self
            .text_bytes
            .checked_add(bytes)
            .ok_or_else(|| MarshalError::new(path, "text size overflow"))?;
        if self.text_bytes > self.limits.max_text_bytes {
            return Err(MarshalError::new(
                path,
                format!("text exceeds {} bytes", self.limits.max_text_bytes),
            ));
        }
        Ok(())
    }
}

/// Serializes an arbitrary Serde value into a bounded Velin value.
///
/// # Errors
/// Returns a path-aware error for Serde failures, unsupported null/floating
/// values, out-of-range integers, or a resource-budget violation.
pub fn to_value<T: Serialize + ?Sized>(
    source: &T,
    limits: MarshallingLimits,
) -> Result<Value, MarshalError> {
    let json = serde_json::to_value(source)
        .map_err(|error| MarshalError::new("$", format!("serialization failed: {error}")))?;
    json_to_value(&json, limits)
}

/// Deserializes a bounded Velin value into an arbitrary owned Serde type.
///
/// # Errors
/// Returns a path-aware error when the value exceeds the supplied budgets or
/// cannot be represented by the requested type.
pub fn from_value<T: DeserializeOwned>(
    value: &Value,
    limits: MarshallingLimits,
) -> Result<T, MarshalError> {
    let json = value_to_json(value, limits)?;
    serde_path_to_error::deserialize(json.into_deserializer()).map_err(|error| {
        let path = error.path().to_string();
        MarshalError::new(
            if path.is_empty() {
                "$".to_owned()
            } else {
                format!("$.{path}")
            },
            format!("deserialization failed: {}", error.inner()),
        )
    })
}

/// Converts a JSON data tree into a bounded Velin value.
///
/// # Errors
/// Returns a path-aware error for unsupported JSON values, integer overflow,
/// or a resource-budget violation.
pub fn json_to_value(value: &JsonValue, limits: MarshallingLimits) -> Result<Value, MarshalError> {
    json_to_value_at(value, "$", 0, &mut Budget::new(limits))
}

/// Converts a Velin value into a bounded, stably ordered JSON data tree.
///
/// # Errors
/// Returns a path-aware error when the value exceeds the supplied budgets.
pub fn value_to_json(value: &Value, limits: MarshallingLimits) -> Result<JsonValue, MarshalError> {
    value_to_json_at(value, "$", 0, &mut Budget::new(limits))
}

fn json_to_value_at(
    value: &JsonValue,
    path: &str,
    depth: usize,
    budget: &mut Budget,
) -> Result<Value, MarshalError> {
    budget.value(path, depth)?;
    match value {
        JsonValue::Null => Err(MarshalError::new(path, "null is not a Velin value")),
        JsonValue::Bool(value) => Ok(Value::Boolean(*value)),
        JsonValue::Number(value) => value.as_i64().map(Value::Integer).ok_or_else(|| {
            MarshalError::new(
                path,
                if value.is_f64() {
                    "floating-point numbers are not supported"
                } else {
                    "integer is outside the i64 range"
                },
            )
        }),
        JsonValue::String(value) => {
            budget.text(path, value.len())?;
            Ok(Value::String(value.clone().into()))
        }
        JsonValue::Array(values) => values
            .iter()
            .enumerate()
            .map(|(index, value)| {
                json_to_value_at(value, &index_path(path, index), depth + 1, budget)
            })
            .collect::<Result<Vec<_>, _>>()
            .map(Arc::new)
            .map(Value::List),
        JsonValue::Object(values) => {
            let mut output = BTreeMap::new();
            for (key, value) in values {
                let field_path = field_path(path, key);
                budget.text(&field_path, key.len())?;
                output.insert(
                    key.clone(),
                    json_to_value_at(value, &field_path, depth + 1, budget)?,
                );
            }
            Ok(Value::Record(Arc::new(output)))
        }
    }
}

fn value_to_json_at(
    value: &Value,
    path: &str,
    depth: usize,
    budget: &mut Budget,
) -> Result<JsonValue, MarshalError> {
    budget.value(path, depth)?;
    match value {
        Value::Integer(value) => Ok(JsonValue::from(*value)),
        Value::Boolean(value) => Ok(JsonValue::from(*value)),
        Value::String(value) => {
            budget.text(path, value.len())?;
            Ok(JsonValue::from(value.as_str()))
        }
        Value::List(values) => values
            .iter()
            .enumerate()
            .map(|(index, value)| {
                value_to_json_at(value, &index_path(path, index), depth + 1, budget)
            })
            .collect::<Result<Vec<_>, _>>()
            .map(JsonValue::Array),
        Value::Record(values) => {
            let mut output = serde_json::Map::new();
            for (key, value) in values.iter() {
                let field_path = field_path(path, key);
                budget.text(&field_path, key.len())?;
                output.insert(
                    key.clone(),
                    value_to_json_at(value, &field_path, depth + 1, budget)?,
                );
            }
            Ok(JsonValue::Object(output))
        }
    }
}

fn index_path(path: &str, index: usize) -> String {
    format!("{path}[{index}]")
}

fn field_path(path: &str, field: &str) -> String {
    let encoded = serde_json::to_string(field).unwrap_or_else(|_| "\"?\"".to_owned());
    format!("{path}[{encoded}]")
}

#[cfg(test)]
#[path = "marshal_tests.rs"]
mod tests;
