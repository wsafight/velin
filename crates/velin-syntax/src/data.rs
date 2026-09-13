use crate::expr::Value;
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

/// Maximum number of values in one validated value tree, including containers.
pub const MAX_DATA_VALUES: usize = 4096;
/// Maximum collection nesting depth in one validated value tree.
pub const MAX_DATA_DEPTH: usize = 16;
/// Maximum UTF-8 text bytes in one validated value tree or rendered string.
pub const MAX_DATA_TEXT_BYTES: usize = 1024 * 1024;

/// Logical size of one value tree.
///
/// Shared collection storage is counted each time it is referenced. This keeps
/// resource accounting deterministic and independent of `Arc` ownership.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DataFootprint {
    pub values: usize,
    pub text_bytes: usize,
}

/// Validated resource metrics for a value tree.
///
/// `max_depth` is relative to the value root, whose depth is zero. Retaining
/// it alongside the footprint lets ownership-aware collection updates prove
/// that a new child still fits without rescanning the existing collection.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DataMetrics {
    pub footprint: DataFootprint,
    pub max_depth: usize,
}

/// The closed set of built-in functions available to expressions.
///
/// These are deterministic helpers with no host, I/O, network, or foreign-code
/// capability. `Random` and `Chance` are stateful, but their state is threaded
/// explicitly by the evaluator/VM rather than hidden in a global generator.
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Builtin {
    List,
    Record,
    Get,
    Put,
    Push,
    Remove,
    Len,
    Contains,
    Random,
    Chance,
}

impl Builtin {
    #[must_use]
    pub fn named(name: &str) -> Option<Self> {
        Some(match name {
            "list" => Self::List,
            "record" => Self::Record,
            "get" => Self::Get,
            "put" => Self::Put,
            "push" => Self::Push,
            "remove" => Self::Remove,
            "len" => Self::Len,
            "contains" => Self::Contains,
            "random" => Self::Random,
            "chance" => Self::Chance,
            _ => return None,
        })
    }
    #[must_use]
    pub const fn accepts(self, count: usize) -> bool {
        match self {
            Self::List => count <= 128,
            Self::Record => count <= 128 && count.is_multiple_of(2),
            Self::Get => count == 2 || count == 3,
            Self::Put => count == 3,
            Self::Push | Self::Remove | Self::Contains | Self::Random => count == 2,
            Self::Len | Self::Chance => count == 1,
        }
    }
}

impl Value {
    /// Validates the deterministic data budget for one variable.
    /// # Errors
    /// Rejects more than 4096 values, 16 collection levels or 1 MiB of text.
    pub fn validate_data(&self) -> Result<(), &'static str> {
        self.data_footprint().map(|_| ())
    }

    /// Measures this value while enforcing the per-value data budget.
    ///
    /// Hosts and the VM use the returned footprint to enforce aggregate
    /// budgets without giving up the existing per-value limits.
    ///
    /// # Errors
    /// Rejects more than 4096 values, 16 collection levels or 1 MiB of text.
    pub fn data_footprint(&self) -> Result<DataFootprint, &'static str> {
        self.data_metrics().map(|metrics| metrics.footprint)
    }

    /// Measures count, text, and nesting depth while enforcing the data budget.
    ///
    /// # Errors
    /// Rejects more than 4096 values, 16 collection levels or 1 MiB of text.
    pub fn data_metrics(&self) -> Result<DataMetrics, &'static str> {
        match self {
            Self::Integer(_) | Self::Boolean(_) => {
                return Ok(DataMetrics {
                    footprint: DataFootprint {
                        values: 1,
                        text_bytes: 0,
                    },
                    max_depth: 0,
                });
            }
            Self::String(text) => {
                if text.len() > MAX_DATA_TEXT_BYTES {
                    return Err("data text exceeds 1 MiB");
                }
                return Ok(DataMetrics {
                    footprint: DataFootprint {
                        values: 1,
                        text_bytes: text.len(),
                    },
                    max_depth: 0,
                });
            }
            Self::List(_) | Self::Record(_) => {}
        }

        let mut pending = vec![(self, 0)];
        let mut items: usize = 0;
        let mut bytes: usize = 0;
        let mut max_depth = 0;
        while let Some((value, depth)) = pending.pop() {
            items = items.checked_add(1).ok_or("data value count overflow")?;
            if items > MAX_DATA_VALUES || depth > MAX_DATA_DEPTH {
                return Err("data exceeds 4096 values or 16 nesting levels");
            }
            max_depth = max_depth.max(depth);
            match value {
                Self::String(text) => {
                    bytes = bytes
                        .checked_add(text.len())
                        .ok_or("data text size overflow")?;
                }
                Self::List(values) => pending.extend(values.iter().map(|value| (value, depth + 1))),
                Self::Record(values) => {
                    for key in values.keys() {
                        bytes = bytes
                            .checked_add(key.len())
                            .ok_or("data text size overflow")?;
                    }
                    pending.extend(values.values().map(|value| (value, depth + 1)));
                }
                Self::Integer(_) | Self::Boolean(_) => {}
            }
            if bytes > MAX_DATA_TEXT_BYTES {
                return Err("data text exceeds 1 MiB");
            }
        }
        Ok(DataMetrics {
            footprint: DataFootprint {
                values: items,
                text_bytes: bytes,
            },
            max_depth,
        })
    }
}

#[cfg(test)]
#[path = "data_tests.rs"]
mod tests;
