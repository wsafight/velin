//! Prevalidated initial variable frames for repeated program instantiation.

use crate::SlotTable;
use std::fmt;
use velin_syntax::{DataFootprint, DataMetrics, Value};

/// A dense, prevalidated variable frame that can be cloned when starting a VM.
///
/// Surface-language defaults are immutable in the common case. Preparing their
/// slot locations and resource metrics once avoids repeating name lookup and
/// recursive value measurement for every script run.
#[derive(Debug, Clone)]
pub struct InitialFrame {
    values: Box<[Option<Value>]>,
    footprints: Box<[DataFootprint]>,
    depths: Box<[u8]>,
    total: DataFootprint,
    bindings: Box<[u32]>,
    layout_id: u64,
}

/// One validated value resolved to its destination frame slot.
///
/// The fields remain private so an [`InitialFrame`] can safely reuse the
/// cached metrics without trusting caller-supplied resource accounting.
#[derive(Debug, Clone)]
pub struct InitialValue {
    slot: u32,
    value: Value,
    metrics: DataMetrics,
}

/// A failure while preparing an [`InitialFrame`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InitialFrameError {
    pub name: String,
    pub message: &'static str,
}

impl fmt::Display for InitialFrameError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "cannot prepare initial value `{}`: {}",
            self.name, self.message
        )
    }
}

impl std::error::Error for InitialFrameError {}

impl InitialValue {
    /// Validates a value once and associates it with an already resolved slot.
    ///
    /// # Errors
    /// Returns an error when `value` exceeds the per-value data budget.
    pub fn new(name: &str, slot: u32, value: Value) -> Result<Self, InitialFrameError> {
        let metrics = value.data_metrics().map_err(|message| InitialFrameError {
            name: name.to_owned(),
            message,
        })?;
        Ok(Self {
            slot,
            value,
            metrics,
        })
    }
}

impl InitialFrame {
    /// Builds a dense frame from values whose slots and metrics were prepared
    /// during surface-language lowering.
    ///
    /// # Errors
    /// Returns an error for a missing or duplicate slot, or aggregate
    /// footprint arithmetic overflow.
    pub fn from_initial_values(
        slots: &SlotTable,
        values: impl IntoIterator<Item = InitialValue>,
    ) -> Result<Self, InitialFrameError> {
        let width = slots.len();
        let mut frame_values = vec![None; width];
        let mut footprints = vec![DataFootprint::default(); width];
        let mut depths = vec![0; width];
        let mut total = DataFootprint::default();
        let mut bindings = Vec::new();

        for initial in values {
            let name = || slots.name(initial.slot).unwrap_or("<unknown>").to_owned();
            let index = initial.slot as usize;
            if index >= width {
                return Err(InitialFrameError {
                    name: name(),
                    message: "unknown variable slot",
                });
            }
            if frame_values[index].is_some() {
                return Err(InitialFrameError {
                    name: name(),
                    message: "duplicate initial value",
                });
            }
            total.values = total
                .values
                .checked_add(initial.metrics.footprint.values)
                .ok_or_else(|| InitialFrameError {
                    name: name(),
                    message: "initial value count overflow",
                })?;
            total.text_bytes = total
                .text_bytes
                .checked_add(initial.metrics.footprint.text_bytes)
                .ok_or_else(|| InitialFrameError {
                    name: name(),
                    message: "initial text size overflow",
                })?;
            frame_values[index] = Some(initial.value);
            footprints[index] = initial.metrics.footprint;
            depths[index] =
                u8::try_from(initial.metrics.max_depth).map_err(|_| InitialFrameError {
                    name: name(),
                    message: "initial value depth overflow",
                })?;
            bindings.push(initial.slot);
        }
        bindings.sort_unstable_by(|left, right| slots.name(*left).cmp(&slots.name(*right)));

        Ok(Self {
            values: frame_values.into_boxed_slice(),
            footprints: footprints.into_boxed_slice(),
            depths: depths.into_boxed_slice(),
            total,
            bindings: bindings.into_boxed_slice(),
            layout_id: slots.layout_id(),
        })
    }

    /// Resolves and validates named values against `slots`, producing dense
    /// frame storage and cached resource metrics.
    ///
    /// # Errors
    /// Returns an error for an unknown or duplicate name, an invalid value, or
    /// aggregate footprint arithmetic overflow.
    pub fn from_named_values<'a>(
        slots: &SlotTable,
        values: impl IntoIterator<Item = (&'a str, &'a Value)>,
    ) -> Result<Self, InitialFrameError> {
        let mut initial_values = Vec::new();
        for (name, value) in values {
            let slot = slots.get(name).ok_or_else(|| InitialFrameError {
                name: name.to_owned(),
                message: "unknown variable",
            })?;
            initial_values.push(InitialValue::new(name, slot, value.clone())?);
        }
        Self::from_initial_values(slots, initial_values)
    }

    /// Returns whether named values still exactly match the source used to
    /// prepare this frame.
    pub fn matches_named_values<'a>(
        &self,
        slots: &SlotTable,
        values: impl IntoIterator<Item = (&'a str, &'a Value)>,
    ) -> bool {
        if !self.matches_layout(slots) {
            return false;
        }
        let mut values = values.into_iter();
        let matches = self.bindings.iter().all(|slot| {
            values.next().is_some_and(|(name, value)| {
                slots.name(*slot) == Some(name)
                    && self.values[*slot as usize].as_ref() == Some(value)
            })
        });
        matches && values.next().is_none()
    }

    /// Returns whether this frame was prepared for the exact slot layout.
    #[must_use]
    pub fn matches_layout(&self, slots: &SlotTable) -> bool {
        self.values.len() == slots.len() && self.layout_id == slots.layout_id()
    }

    #[must_use]
    pub fn values(&self) -> &[Option<Value>] {
        &self.values
    }

    #[must_use]
    pub fn footprints(&self) -> &[DataFootprint] {
        &self.footprints
    }

    #[must_use]
    pub fn depths(&self) -> &[u8] {
        &self.depths
    }

    #[must_use]
    pub const fn total(&self) -> DataFootprint {
        self.total
    }

    /// Returns the slots populated by this frame, ordered by variable name.
    #[must_use]
    pub fn assigned_slots(&self) -> &[u32] {
        &self.bindings
    }
}

#[cfg(test)]
#[path = "frame_tests.rs"]
mod tests;
