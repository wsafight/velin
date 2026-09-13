use super::*;

#[test]
fn scalar_metrics_are_minimal() {
    assert_eq!(
        scalar_metrics(),
        DataMetrics {
            footprint: DataFootprint {
                values: 1,
                text_bytes: 0,
            },
            max_depth: 0,
        }
    );
}

#[test]
fn collection_metrics_sum_children_and_enforce_budgets() {
    let scalar = scalar_metrics();
    assert_eq!(
        collection_metrics(&[scalar, scalar], 1).unwrap(),
        DataMetrics {
            footprint: DataFootprint {
                values: 3,
                text_bytes: 0,
            },
            max_depth: 1,
        }
    );
    let deep = DataMetrics {
        max_depth: MAX_DATA_DEPTH,
        ..scalar_metrics()
    };
    assert!(collection_metrics(&[deep], 1).is_err());
    let huge = DataMetrics {
        footprint: DataFootprint {
            values: MAX_DATA_VALUES,
            text_bytes: 0,
        },
        max_depth: 0,
    };
    assert!(collection_metrics(&[huge; 2], 1).is_err());
    let text = DataMetrics {
        footprint: DataFootprint {
            values: 1,
            text_bytes: MAX_DATA_TEXT_BYTES,
        },
        max_depth: 0,
    };
    assert!(collection_metrics(&[text; 2], 1).is_err());
}

#[test]
fn record_metrics_decline_non_string_keys() {
    let metrics = [scalar_metrics(), scalar_metrics()];
    assert!(record_metrics(&[Value::Integer(1), Value::Integer(2)], &metrics, 1).is_none());
    let record = record_metrics(
        &[Value::String("k".into()), Value::String("v".into())],
        &[scalar_metrics(), scalar_metrics()],
        1,
    )
    .unwrap()
    .unwrap();
    assert_eq!(record.footprint.values, 2);
    assert_eq!(record.footprint.text_bytes, 1);
}

#[test]
fn push_metrics_require_two_child_metrics() {
    assert!(push_metrics(&[scalar_metrics()], 1).is_none());
    assert!(
        push_metrics(&[scalar_metrics(), scalar_metrics()], 1)
            .unwrap()
            .is_ok()
    );
}

#[test]
fn validate_metrics_reject_each_budget() {
    let values = DataMetrics {
        footprint: DataFootprint {
            values: MAX_DATA_VALUES + 1,
            text_bytes: 0,
        },
        max_depth: 0,
    };
    assert!(validate_metrics(values, 1).is_err());
    let text = DataMetrics {
        footprint: DataFootprint {
            values: 1,
            text_bytes: MAX_DATA_TEXT_BYTES + 1,
        },
        max_depth: 0,
    };
    assert!(validate_metrics(text, 1).is_err());
    assert!(validate_metrics(scalar_metrics(), 1).is_ok());
}
