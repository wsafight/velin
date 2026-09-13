use super::*;

#[test]
fn prepares_dense_metrics_and_detects_source_changes() {
    let mut slots = SlotTable::new();
    slots.intern("unused");
    slots.intern("hp");
    let hp = Value::Integer(3);
    let frame = InitialFrame::from_named_values(&slots, [("hp", &hp)]).unwrap();
    assert_eq!(frame.values(), &[None, Some(Value::Integer(3))]);
    assert_eq!(frame.total().values, 1);
    assert!(frame.matches_named_values(&slots, [("hp", &hp)]));
    assert!(!frame.matches_named_values(&slots, [("hp", &Value::Integer(4))]));
    assert!(!frame.matches_named_values(&slots, [("other", &hp)]));
}

#[test]
fn reports_every_initial_frame_error_shape() {
    let mut slots = SlotTable::new();
    slots.intern("hp");
    slots.intern("mp");

    // An unknown slot id is rejected before anything else is measured.
    let error = InitialFrame::from_initial_values(
        &slots,
        [InitialValue::new("ghost", 9, Value::Integer(1)).unwrap()],
    )
    .unwrap_err();
    assert_eq!(error.message, "unknown variable slot");
    assert!(error.to_string().contains("cannot prepare initial value"));

    let error =
        InitialFrame::from_named_values(&slots, [("missing", &Value::Integer(1))]).unwrap_err();
    assert_eq!(error.message, "unknown variable");
    assert_eq!(error.name, "missing");
    let named = InitialFrame::from_named_values(
        &slots,
        [("hp", &Value::Integer(5)), ("mp", &Value::Integer(6))],
    )
    .unwrap();
    assert_eq!(named.total().values, 2);
    assert_eq!(named.values()[0], Some(Value::Integer(5)));
    assert_eq!(named.values()[1], Some(Value::Integer(6)));

    // Duplicate slots are detected after the first is installed.
    let first = InitialValue::new("hp", 0, Value::Integer(1)).unwrap();
    let second = InitialValue::new("hp", 0, Value::Integer(2)).unwrap();
    let error = InitialFrame::from_initial_values(&slots, [first, second]).unwrap_err();
    assert_eq!(error.message, "duplicate initial value");

    // A value that exceeds the per-value budget fails during `InitialValue`.
    let huge = Value::String("x".repeat(velin_syntax::MAX_DATA_TEXT_BYTES + 1).into());
    let error = InitialValue::new("hp", 0, huge).unwrap_err();
    assert_eq!(error.message, "data text exceeds 1 MiB");
}

#[test]
fn copies_named_values_into_dense_storage() {
    let mut slots = SlotTable::new();
    slots.intern("hp");
    let frame = InitialFrame::from_named_values(&slots, [("hp", &Value::Integer(5))]).unwrap();
    assert_eq!(frame.values()[0], Some(Value::Integer(5)));
    assert_eq!(frame.total().values, 1);
    let empty = InitialFrame::from_named_values(&slots, []).unwrap();
    assert_eq!(empty.values(), &[None]);
    assert!(empty.assigned_slots().is_empty());
}

#[test]
fn orders_bindings_by_name_and_reports_dense_metrics() {
    let mut slots = SlotTable::new();
    slots.intern("zeta");
    slots.intern("alpha");
    let frame = InitialFrame::from_named_values(
        &slots,
        [
            ("zeta", &Value::Integer(1)),
            ("alpha", &Value::String("a".into())),
        ],
    )
    .unwrap();
    assert_eq!(frame.assigned_slots(), &[1, 0]);
    assert_eq!(frame.total().values, 2);
    assert_eq!(frame.total().text_bytes, 1);
    assert_eq!(frame.depths(), &[0, 0]);
    assert_eq!(frame.footprints()[0].values, 1);
    assert_eq!(frame.values()[0], Some(Value::Integer(1)));
    assert!(frame.matches_named_values(
        &slots,
        [
            ("alpha", &Value::String("a".into())),
            ("zeta", &Value::Integer(1))
        ]
    ));
    assert!(!frame.matches_named_values(&slots, [("alpha", &Value::String("a".into()))]));
}
