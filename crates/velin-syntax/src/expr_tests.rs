use super::*;

#[test]
fn spans_record_source_line_and_column() {
    let span = Span::new(3, 8);
    assert_eq!(span.line, 3);
    assert_eq!(span.column, 8);
    assert_eq!(span.source, "");
    let located = Span::in_source("script.velin", 1, 2);
    assert_eq!(located.source, "script.velin");
    assert_eq!(located.line, 1);
    assert_eq!(located.column, 2);
}

#[test]
fn value_type_names_are_stable() {
    assert_eq!(Value::Integer(1).type_name(), "integer");
    assert_eq!(Value::Boolean(true).type_name(), "boolean");
    assert_eq!(Value::String(String::new().into()).type_name(), "string");
    assert_eq!(
        Value::List(std::sync::Arc::new(Vec::new())).type_name(),
        "list"
    );
    assert_eq!(
        Value::Record(std::sync::Arc::new(std::collections::BTreeMap::new())).type_name(),
        "record"
    );
}

#[test]
fn to_display_is_deterministic_and_locale_free() {
    assert_eq!(Value::Integer(-42).to_display(), "-42");
    assert_eq!(Value::Boolean(false).to_display(), "false");
    assert_eq!(Value::String("hi".into()).to_display(), "hi");
    let list = Value::List(std::sync::Arc::new(vec![
        Value::Integer(1),
        Value::String("x".into()),
    ]));
    assert_eq!(list.to_display(), "[1, x]");
    let record = Value::Record(std::sync::Arc::new(std::collections::BTreeMap::from([
        ("b".to_owned(), Value::Integer(2)),
        ("a".to_owned(), Value::Boolean(true)),
    ])));
    // BTreeMap keeps keys sorted, so the rendering is stable.
    assert_eq!(record.to_display(), "{a: true, b: 2}");
}

#[test]
fn bounded_display_counts_collection_punctuation() {
    let value = Value::List(std::sync::Arc::new(vec![Value::String(
        "x".repeat(crate::data::MAX_DATA_TEXT_BYTES).into(),
    )]));
    assert!(value.validate_data().is_ok());
    assert_eq!(value.try_to_display(), Err("rendered text exceeds 1 MiB"));
}

#[test]
fn display_length_matches_rendered_bytes_without_scalar_temporaries() {
    let value = Value::Record(std::sync::Arc::new(std::collections::BTreeMap::from([
        ("answer".to_owned(), Value::Integer(i64::MIN)),
        ("ok".to_owned(), Value::Boolean(true)),
    ])));
    let rendered = value.to_display();
    assert_eq!(value.display_len().unwrap(), rendered.len());
    let mut output = String::new();
    value
        .append_to_display_known(&mut output, crate::data::MAX_DATA_TEXT_BYTES)
        .unwrap();
    assert_eq!(output, rendered);
}
