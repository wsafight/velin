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

#[test]
fn names_can_be_dropped_without_changing_slot_width_or_rng_slot() {
    let mut slots = SlotTable::new();
    let value = slots.intern("value");
    let rng = slots.intern_rng_state();
    let stripped = slots.without_names();
    assert_eq!(stripped.len(), 2);
    assert!(!stripped.has_names());
    assert_eq!(stripped.get("value"), None);
    assert_eq!(stripped.name(value), None);
    assert_eq!(stripped.rng_state(), Some(rng));
}
