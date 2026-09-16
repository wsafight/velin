use super::*;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
struct Payload {
    name: String,
    values: Vec<i64>,
    flags: BTreeMap<String, bool>,
}

#[test]
fn serde_values_round_trip_with_stable_records() {
    let payload = Payload {
        name: "sample".to_owned(),
        values: vec![1, 2, 3],
        flags: BTreeMap::from([("z".to_owned(), false), ("a".to_owned(), true)]),
    };
    let value = to_value(&payload, MarshallingLimits::default()).unwrap();
    let Value::Record(fields) = &value else {
        panic!("expected record")
    };
    assert_eq!(fields.keys().next().map(String::as_str), Some("flags"));
    assert_eq!(
        from_value::<Payload>(&value, MarshallingLimits::default()).unwrap(),
        payload
    );
}

#[test]
fn unsupported_values_and_type_errors_keep_precise_paths() {
    let json = serde_json::json!({ "items": [1, null] });
    let error = json_to_value(&json, MarshallingLimits::default()).unwrap_err();
    assert_eq!(error.path(), "$[\"items\"][1]");
    assert!(error.message().contains("null"));

    let json = serde_json::json!({ "items": [1.5] });
    let error = json_to_value(&json, MarshallingLimits::default()).unwrap_err();
    assert_eq!(error.path(), "$[\"items\"][0]");
    assert!(error.message().contains("floating-point"));

    let value = Value::Record(Arc::new(BTreeMap::from([(
        "values".to_owned(),
        Value::List(Arc::new(vec![Value::String("wrong".into())])),
    )])));
    let error = from_value::<Payload>(&value, MarshallingLimits::default()).unwrap_err();
    assert!(error.path().contains("values[0]"), "{error}");
}

#[test]
fn every_budget_reports_the_value_that_crossed_it() {
    let values = serde_json::json!([1, 2]);
    let error = json_to_value(
        &values,
        MarshallingLimits {
            max_values: 2,
            ..MarshallingLimits::default()
        },
    )
    .unwrap_err();
    assert_eq!(error.path(), "$[1]");

    let nested = serde_json::json!([[1]]);
    let error = json_to_value(
        &nested,
        MarshallingLimits {
            max_depth: 1,
            ..MarshallingLimits::default()
        },
    )
    .unwrap_err();
    assert_eq!(error.path(), "$[0][0]");

    let text = serde_json::json!({ "key": "value" });
    let error = json_to_value(
        &text,
        MarshallingLimits {
            max_text_bytes: 5,
            ..MarshallingLimits::default()
        },
    )
    .unwrap_err();
    assert_eq!(error.path(), "$[\"key\"]");
}
