//! Compile-time mapping between variable names and dense frame slots.
//!
//! The tree-walker looks variables up by string in a `BTreeMap`. The bytecode
//! path resolves every name to a small integer index at compile time, so the
//! VM can use a flat `Vec<Value>` frame with no hashing at runtime.

use std::collections::HashMap;

/// Reserved slot name used to thread deterministic RNG state through a frame.
///
/// The NUL prefix cannot be produced by Velin's identifier grammar, so script
/// variables cannot collide with this internal-but-serializable slot.
pub const RNG_STATE_SLOT: &str = "\0velin_rng_state";

/// A stable, compile-time assignment of variable names to frame indices.
///
/// Interning is monotonic: a name always maps to the same slot for the life of
/// the table, and the reverse mapping (`name`) is kept for diagnostics, save
/// files, and text interpolation on the host side.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlotTable {
    names: Option<Vec<String>>,
    index: Option<HashMap<String, u32>>,
    width: usize,
    rng_state: Option<u32>,
}

impl Default for SlotTable {
    fn default() -> Self {
        Self {
            names: Some(Vec::new()),
            index: Some(HashMap::new()),
            width: 0,
            rng_state: None,
        }
    }
}

impl SlotTable {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the slot for `name`, allocating a new one on first use.
    ///
    /// # Panics
    /// Panics only if more than `u32::MAX` distinct names are interned, which
    /// the data budget makes unreachable in practice.
    pub fn intern(&mut self, name: &str) -> u32 {
        let index = self
            .index
            .as_ref()
            .expect("cannot intern into a name-stripped slot table");
        if let Some(slot) = index.get(name) {
            return *slot;
        }
        let slot = u32::try_from(self.width).expect("slot count fits in u32");
        self.names
            .as_mut()
            .expect("name index and names must be present")
            .push(name.to_owned());
        self.index
            .as_mut()
            .expect("name index and names must be present")
            .insert(name.to_owned(), slot);
        self.width += 1;
        if name == RNG_STATE_SLOT {
            self.rng_state = Some(slot);
        }
        slot
    }

    /// Returns the dedicated RNG state slot, allocating it on first use.
    pub fn intern_rng_state(&mut self) -> u32 {
        if let Some(slot) = self.rng_state {
            return slot;
        }
        self.intern(RNG_STATE_SLOT)
    }

    /// Returns the RNG state slot when this program contains random operations.
    #[must_use]
    pub fn rng_state(&self) -> Option<u32> {
        self.rng_state
    }

    /// Returns the slot for `name` if it has already been interned.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<u32> {
        self.index.as_ref()?.get(name).copied()
    }

    /// Returns the name bound to `slot`, if any.
    #[must_use]
    pub fn name(&self, slot: u32) -> Option<&str> {
        self.names.as_ref()?.get(slot as usize).map(String::as_str)
    }

    /// The number of distinct slots, i.e. the frame width the VM must allocate.
    #[must_use]
    pub fn len(&self) -> usize {
        self.width
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.width == 0
    }

    /// All slot names in slot order; index `i` is the name of slot `i`.
    #[must_use]
    pub fn names(&self) -> &[String] {
        self.names.as_deref().unwrap_or_default()
    }

    /// Returns whether this table retains names and name-to-slot lookup.
    #[must_use]
    pub fn has_names(&self) -> bool {
        self.names.is_some()
    }

    /// Drops names and reverse lookup while preserving slot IDs and width.
    ///
    /// The resulting table is suitable for hosts that only execute bytecode
    /// by numeric slot ID. Name-based binding and diagnostics intentionally
    /// become unavailable, while the reserved RNG slot remains addressable.
    #[must_use]
    pub fn without_names(mut self) -> Self {
        self.names = None;
        self.index = None;
        self
    }

    #[cfg(feature = "serde")]
    pub(crate) fn from_nameless(
        width: usize,
        rng_state: Option<u32>,
    ) -> Result<Self, &'static str> {
        if rng_state.is_some_and(|slot| slot as usize >= width) {
            return Err("RNG slot is outside the nameless slot width");
        }
        Ok(Self {
            names: None,
            index: None,
            width,
            rng_state,
        })
    }
}

#[cfg(test)]
#[path = "slots_tests.rs"]
mod tests;
