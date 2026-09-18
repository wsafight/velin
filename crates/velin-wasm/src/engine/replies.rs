//! Bounded JSON parsing for scripted Playground host replies.

use std::fmt;
use velin::{MarshallingLimits, Value, json_to_value};

pub(super) const MAX_REPLIES_JSON_BYTES: usize = 1024 * 1024;

/// A caller error in the scripted replies supplied to the Playground host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseRepliesError {
    TooLarge,
    InvalidJson(String),
    UnsupportedValue {
        index: usize,
        path: String,
        message: String,
    },
}

impl fmt::Display for ParseRepliesError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooLarge => write!(formatter, "invalid replies: JSON exceeds 1 MiB"),
            Self::InvalidJson(message) => write!(formatter, "invalid replies JSON: {message}"),
            Self::UnsupportedValue {
                index,
                path,
                message,
            } => write!(
                formatter,
                "invalid replies: item {index} at {path}: {message}"
            ),
        }
    }
}

/// Parses the caller's JSON reply array into Velin values.
///
/// The entire input is rejected if any item is unsupported. Silently dropping
/// an item would shift every subsequent answer to the wrong `ask` effect.
pub fn parse_replies(replies_json: &str) -> Result<Vec<Value>, ParseRepliesError> {
    if replies_json.len() > MAX_REPLIES_JSON_BYTES {
        return Err(ParseRepliesError::TooLarge);
    }
    let parsed = serde_json::from_str::<Vec<serde_json::Value>>(replies_json)
        .map_err(|error| ParseRepliesError::InvalidJson(error.to_string()))?;
    parsed
        .iter()
        .enumerate()
        .map(|(index, value)| {
            json_to_value(value, MarshallingLimits::default()).map_err(|error| {
                ParseRepliesError::UnsupportedValue {
                    index: index + 1,
                    path: error.path().to_owned(),
                    message: error.message().to_owned(),
                }
            })
        })
        .collect()
}
