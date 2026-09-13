use super::*;
use crate::expr::Value;
use std::collections::BTreeMap;
use std::sync::Arc;

#[test]
fn names_and_argument_counts_match_the_data_builtins() {
    assert_eq!(Builtin::named("list"), Some(Builtin::List));
    assert_eq!(Builtin::named("contains"), Some(Builtin::Contains));
    assert_eq!(Builtin::named("random"), Some(Builtin::Random));
    assert_eq!(Builtin::named("chance"), Some(Builtin::Chance));
    assert_eq!(Builtin::named("unknown"), None);
    assert!(Builtin::List.accepts(0));
    assert!(Builtin::Record.accepts(2));
    assert!(!Builtin::Record.accepts(1));
    assert!(Builtin::Get.accepts(3));
    assert!(!Builtin::Len.accepts(2));
    assert!(Builtin::Random.accepts(2));
    assert!(Builtin::Chance.accepts(1));
    assert!(Builtin::Push.accepts(2));
    assert!(Builtin::Remove.accepts(2));
    assert!(Builtin::Contains.accepts(2));
    assert!(Builtin::Put.accepts(3));
    assert_eq!(Builtin::named("record"), Some(Builtin::Record));
    assert_eq!(Builtin::named("get"), Some(Builtin::Get));
    assert_eq!(Builtin::named("put"), Some(Builtin::Put));
    assert_eq!(Builtin::named("push"), Some(Builtin::Push));
    assert_eq!(Builtin::named("remove"), Some(Builtin::Remove));
    assert_eq!(Builtin::named("len"), Some(Builtin::Len));
}

#[test]
fn data_budget_rejects_wide_deep_and_heavy_values() {
    assert_eq!(
        Value::Integer(1).data_footprint().unwrap(),
        DataFootprint {
            values: 1,
            text_bytes: 0
        }
    );
    assert_eq!(
        Value::String("hello".into()).data_footprint().unwrap(),
        DataFootprint {
            values: 1,
            text_bytes: 5
        }
    );
    let wide = Value::List(Arc::new(vec![Value::Integer(0); 4096]));
    assert!(wide.validate_data().is_err());
    let mut nested = Value::Integer(1);
    for _ in 0..17 {
        nested = Value::List(Arc::new(vec![nested]));
    }
    assert!(nested.validate_data().is_err());
    let heavy = Value::Record(Arc::new(BTreeMap::from([(
        "x".repeat(1024 * 1024 + 1),
        Value::Boolean(true),
    )])));
    assert_eq!(heavy.validate_data(), Err("data text exceeds 1 MiB"));
}
