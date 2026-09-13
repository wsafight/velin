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
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SlotTable {
    names: Vec<String>,
    index: HashMap<String, u32>,
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
        if let Some(slot) = self.index.get(name) {
            return *slot;
        }
        let slot = u32::try_from(self.names.len()).expect("slot count fits in u32");
        self.names.push(name.to_owned());
        self.index.insert(name.to_owned(), slot);
        slot
    }

    /// Returns the dedicated RNG state slot, allocating it on first use.
    pub fn intern_rng_state(&mut self) -> u32 {
        self.intern(RNG_STATE_SLOT)
    }

    /// Returns the RNG state slot when this program contains random operations.
    #[must_use]
    pub fn rng_state(&self) -> Option<u32> {
        self.get(RNG_STATE_SLOT)
    }

    /// Returns the slot for `name` if it has already been interned.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<u32> {
        self.index.get(name).copied()
    }

    /// Returns the name bound to `slot`, if any.
    #[must_use]
    pub fn name(&self, slot: u32) -> Option<&str> {
        self.names.get(slot as usize).map(String::as_str)
    }

    /// The number of distinct slots, i.e. the frame width the VM must allocate.
    #[must_use]
    pub fn len(&self) -> usize {
        self.names.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.names.is_empty()
    }

    /// All slot names in slot order; index `i` is the name of slot `i`.
    #[must_use]
    pub fn names(&self) -> &[String] {
        &self.names
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interning_is_stable_and_reversible() {
        let mut slots = SlotTable::new();
        let hp = slots.intern("hp");
        let mp = slots.intern("mp");
        assert_ne!(hp, mp);
        assert_eq!(slots.intern("hp"), hp);
        assert_eq!(slots.get("mp"), Some(mp));
        assert_eq!(slots.get("missing"), None);
        assert_eq!(slots.name(hp), Some("hp"));
        assert_eq!(slots.len(), 2);
        assert!(!slots.is_empty());
        assert!(SlotTable::new().is_empty());
        assert_eq!(slots.names().len(), 2);
        assert_eq!(slots.name(99), None);
    }
}
