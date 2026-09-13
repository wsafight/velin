use super::*;
use velin_syntax::MAX_DATA_TEXT_BYTES;

#[test]
fn list_and_record_construct_and_read_back() {
    let list = invoke(Builtin::List, vec![Value::Integer(1), Value::Integer(2)], 1).unwrap();
    assert_eq!(
        invoke(Builtin::Len, vec![list.clone()], 1).unwrap(),
        Value::Integer(2)
    );
    let first = invoke(Builtin::Get, vec![list, Value::Integer(0)], 1).unwrap();
    assert_eq!(first, Value::Integer(1));

    let record = invoke(
        Builtin::Record,
        vec![Value::String("done".into()), Value::Boolean(true)],
        1,
    )
    .unwrap();
    let got = invoke(Builtin::Get, vec![record, Value::String("done".into())], 1).unwrap();
    assert_eq!(got, Value::Boolean(true));
}

#[test]
fn push_put_and_remove_are_copy_on_write() {
    let list = Value::List(Arc::new(vec![Value::String("key".into())]));
    let pushed = invoke(
        Builtin::Push,
        vec![list.clone(), Value::String("map".into())],
        1,
    )
    .unwrap();
    // Original is untouched (structural sharing).
    assert_eq!(
        invoke(Builtin::Len, vec![list], 1).unwrap(),
        Value::Integer(1)
    );
    assert_eq!(
        invoke(Builtin::Len, vec![pushed.clone()], 1).unwrap(),
        Value::Integer(2)
    );
    let removed = invoke(Builtin::Remove, vec![pushed, Value::Integer(0)], 1).unwrap();
    assert_eq!(
        invoke(Builtin::Len, vec![removed], 1).unwrap(),
        Value::Integer(1)
    );
}

#[test]
fn invalid_calls_and_collection_growth_are_bounded() {
    assert!(
        invoke(
            Builtin::Get,
            vec![Value::List(Arc::new(vec![])), Value::Integer(-1)],
            1
        )
        .is_err()
    );
    let list = Value::List(Arc::new(vec![Value::Integer(0); 4095]));
    assert!(invoke(Builtin::Push, vec![list, Value::Integer(1)], 1).is_err());
    assert!(
        invoke(
            Builtin::Record,
            vec![Value::Integer(1), Value::Boolean(true)],
            1
        )
        .is_err()
    );
}

#[test]
fn random_calls_are_deterministic_bounded_and_advance_state() {
    let mut first_state = 7;
    let mut second_state = 7;
    let mut first = Vec::new();
    let mut second = Vec::new();
    for _ in 0..32 {
        first.push(
            invoke_random(
                Builtin::Random,
                &[Value::Integer(-3), Value::Integer(4)],
                &mut first_state,
                1,
            )
            .unwrap(),
        );
        second.push(
            invoke_random(
                Builtin::Random,
                &[Value::Integer(-3), Value::Integer(4)],
                &mut second_state,
                1,
            )
            .unwrap(),
        );
    }
    assert_eq!(first, second);
    assert_eq!(first_state, second_state);
    assert!(
        first
            .iter()
            .all(|value| matches!(value, Value::Integer(-3..=4)))
    );
    assert_ne!(first_state, 7);
}

#[test]
fn invalid_random_calls_do_not_advance_state() {
    let mut state = 11;
    assert!(
        invoke_random(
            Builtin::Random,
            &[Value::Integer(3), Value::Integer(2)],
            &mut state,
            1,
        )
        .is_err()
    );
    assert_eq!(state, 11);
    assert!(invoke_random(Builtin::Chance, &[Value::Integer(101)], &mut state, 1,).is_err());
    assert_eq!(state, 11);
}

#[test]
fn random_supports_full_i64_range_and_chance_boundaries() {
    let mut state = i64::MIN;
    let value = invoke_random(
        Builtin::Random,
        &[Value::Integer(i64::MIN), Value::Integer(i64::MAX)],
        &mut state,
        1,
    )
    .unwrap();
    assert!(matches!(value, Value::Integer(_)));
    assert_ne!(state, i64::MIN);

    assert_eq!(
        invoke_random(Builtin::Chance, &[Value::Integer(0)], &mut state, 1,).unwrap(),
        Value::Boolean(false)
    );
    assert_eq!(
        invoke_random(Builtin::Chance, &[Value::Integer(100)], &mut state, 1,).unwrap(),
        Value::Boolean(true)
    );
}

#[test]
fn len_get_contains_and_duplicate_keys_error() {
    assert!(invoke(Builtin::Len, Vec::new(), 1).is_err());
    assert!(invoke(Builtin::Len, vec![Value::Integer(1)], 1).is_err());
    assert_eq!(
        invoke(Builtin::Len, vec![Value::String("ab".into())], 1).unwrap(),
        Value::Integer(2)
    );
    assert!(
        invoke(
            Builtin::Record,
            vec![
                Value::String("a".into()),
                Value::Integer(1),
                Value::String("a".into()),
                Value::Integer(2),
            ],
            1
        )
        .is_err()
    );

    let list = Value::List(Arc::new(vec![Value::Integer(1)]));
    let record = invoke(
        Builtin::Record,
        vec![Value::String("k".into()), Value::Integer(1)],
        1,
    )
    .unwrap();
    assert!(invoke(Builtin::Get, vec![Value::Integer(1), Value::Integer(0)], 1).is_err());
    assert_eq!(
        invoke(
            Builtin::Get,
            vec![record.clone(), Value::String("k".into())],
            1
        )
        .unwrap(),
        Value::Integer(1)
    );
    assert!(
        invoke(
            Builtin::Contains,
            vec![Value::Integer(1), Value::Integer(0)],
            1
        )
        .is_err()
    );
    assert_eq!(
        invoke(
            Builtin::Contains,
            vec![record, Value::String("k".into())],
            1
        )
        .unwrap(),
        Value::Boolean(true)
    );
    assert_eq!(
        invoke(
            Builtin::Contains,
            vec![Value::String("abc".into()), Value::String("b".into())],
            1
        )
        .unwrap(),
        Value::Boolean(true)
    );
    assert_eq!(
        invoke(Builtin::Contains, vec![list, Value::Integer(1)], 1).unwrap(),
        Value::Boolean(true)
    );
    assert!(invoke(Builtin::Push, vec![Value::Integer(1), Value::Integer(2)], 1).is_err());
    assert!(
        invoke(
            Builtin::Random,
            vec![Value::Integer(1), Value::Integer(2)],
            1
        )
        .is_err()
    );
}

#[test]
fn readonly_invocation_matches_owned_builtin_results() {
    let list = Value::List(Arc::new(vec![Value::String("map".into())]));
    let zero = Value::Integer(0);
    let missing = Value::Integer(1);
    let default = Value::String("fallback".into());
    let map = Value::String("map".into());

    assert_eq!(
        invoke_readonly_measured(Builtin::Len, &[&list], 1)
            .unwrap()
            .0,
        Value::Integer(1)
    );
    assert_eq!(
        invoke_readonly_measured(Builtin::Get, &[&list, &zero], 1)
            .unwrap()
            .0,
        map
    );
    assert_eq!(
        invoke_readonly_measured(Builtin::Get, &[&list, &missing, &default], 1)
            .unwrap()
            .0,
        default
    );
    assert_eq!(
        invoke_readonly_measured(Builtin::Contains, &[&list, &map], 1)
            .unwrap()
            .0,
        Value::Boolean(true)
    );
    assert!(invoke_readonly_measured(Builtin::Len, &[], 1).is_err());
    assert!(invoke_readonly_measured(Builtin::Push, &[&list, &map], 1).is_err());
}

#[test]
fn metric_aware_collection_calls_match_value_metrics() {
    let arguments = vec![Value::Integer(1), Value::String("xy".into())];
    let argument_metrics = arguments
        .iter()
        .map(|value| value.data_metrics().unwrap())
        .collect::<Vec<_>>();
    let (list, list_metrics) =
        invoke_measured_with_metrics(Builtin::List, arguments, &argument_metrics, 1).unwrap();
    assert_eq!(list_metrics, list.data_metrics().unwrap());

    let child = Value::List(Arc::new(vec![Value::Boolean(true)]));
    let child_metrics = child.data_metrics().unwrap();
    let (pushed, pushed_metrics) = invoke_measured_with_metrics(
        Builtin::Push,
        vec![list, child],
        &[list_metrics, child_metrics],
        1,
    )
    .unwrap();
    assert_eq!(pushed_metrics, pushed.data_metrics().unwrap());
}

#[test]
fn metric_aware_calls_cover_scalars_records_and_failures() {
    let scalar = scalar_metrics();
    let arguments = vec![Value::String("xy".into()), Value::Integer(1)];
    let argument_metrics = vec![Value::String("xy".into()).data_metrics().unwrap(), scalar];
    let (list, list_metrics) =
        invoke_measured_with_metrics(Builtin::List, arguments, &argument_metrics, 1).unwrap();
    assert_eq!(list_metrics, list.data_metrics().unwrap());

    let (length, length_metrics) =
        invoke_measured_with_metrics(Builtin::Len, vec![list.clone()], &[list_metrics], 1).unwrap();
    assert_eq!(length, Value::Integer(2));
    assert_eq!(length_metrics, length.data_metrics().unwrap());

    assert!(
        invoke_measured_with_metrics(
            Builtin::List,
            vec![Value::Integer(1), Value::Integer(2)],
            &[scalar],
            1,
        )
        .unwrap_err()
        .message
        .contains("argument metrics")
    );

    let (record, record_metrics) = invoke_measured_with_metrics(
        Builtin::Record,
        vec![Value::String("k".into()), Value::Integer(1)],
        &[Value::String("k".into()).data_metrics().unwrap(), scalar],
        1,
    )
    .unwrap();
    assert_eq!(record_metrics, record.data_metrics().unwrap());

    let (pushed, pushed_metrics) = invoke_measured_with_metrics(
        Builtin::Push,
        vec![list, Value::Integer(3)],
        &[list_metrics, scalar],
        1,
    )
    .unwrap();
    assert_eq!(pushed_metrics, pushed.data_metrics().unwrap());
}

#[test]
fn metric_aware_collection_calls_reject_oversized_and_non_string_shapes() {
    let scalar = scalar_metrics();
    // `list` rejects more than its 128-argument limit before measuring.
    assert!(
        invoke_measured_with_metrics(
            Builtin::List,
            vec![Value::Integer(0); 129],
            &vec![scalar; 129],
            1,
        )
        .unwrap_err()
        .message
        .contains("argument count")
    );

    // A record with a non-string key declines the fast path.
    assert!(
        invoke_measured_with_metrics(
            Builtin::Record,
            vec![Value::Integer(1), Value::Integer(2)],
            &[scalar, scalar],
            1,
        )
        .unwrap_err()
        .message
        .contains("must be strings")
    );

    // A text child that exhausts the shared text budget is rejected.
    let text = Value::String("x".repeat(MAX_DATA_TEXT_BYTES).into());
    let text_metrics = text.data_metrics().unwrap();
    assert!(
        invoke_measured_with_metrics(
            Builtin::Record,
            vec![
                Value::String("big".into()),
                text,
                Value::String("b".into()),
                Value::String("c".into()),
            ],
            &[scalar, text_metrics, scalar, scalar],
            1,
        )
        .unwrap_err()
        .message
        .contains("1 MiB")
    );

    // A child list that fills the value budget pushes the aggregate over it.
    let big = Value::List(Arc::new((0..4095).map(Value::Integer).collect::<Vec<_>>()));
    let big_metrics = big.data_metrics().unwrap();
    assert!(
        invoke_measured_with_metrics(
            Builtin::Record,
            vec![Value::String("k".into()), big],
            &[scalar, big_metrics],
            1,
        )
        .unwrap_err()
        .message
        .contains("4096")
    );

    // `push` declines when the fast path cannot prove the list shape.
    assert!(
        invoke_measured_with_metrics(
            Builtin::Push,
            vec![Value::Integer(1), Value::Integer(2)],
            &[scalar, scalar],
            1,
        )
        .unwrap_err()
        .message
        .contains("push expects a list")
    );

    let (empty_record, empty_metrics) =
        invoke_measured_with_metrics(Builtin::Record, Vec::new(), &[], 1).unwrap();
    assert_eq!(empty_metrics, empty_record.data_metrics().unwrap());
}

#[test]
fn record_edits_and_random_arity_errors() {
    let record = invoke(
        Builtin::Record,
        vec![Value::String("k".into()), Value::Integer(1)],
        1,
    )
    .unwrap();
    let updated = invoke(
        Builtin::Put,
        vec![record.clone(), Value::String("k".into()), Value::Integer(9)],
        1,
    )
    .unwrap();
    assert_eq!(
        invoke(
            Builtin::Get,
            vec![updated.clone(), Value::String("k".into())],
            1
        )
        .unwrap(),
        Value::Integer(9)
    );
    let removed = invoke(Builtin::Remove, vec![updated, Value::String("k".into())], 1).unwrap();
    assert!(invoke(Builtin::Remove, vec![removed, Value::String("k".into())], 1).is_err());
    assert!(
        invoke(
            Builtin::Put,
            vec![Value::Integer(1), Value::Integer(0), Value::Integer(2)],
            1
        )
        .is_err()
    );

    let mut state = 1;
    assert!(invoke_random(Builtin::Random, &[], &mut state, 1).is_err());
    assert!(
        invoke_random(
            Builtin::Random,
            &[Value::Boolean(true), Value::Integer(1)],
            &mut state,
            1
        )
        .is_err()
    );
    assert!(invoke_random(Builtin::Chance, &[], &mut state, 1).is_err());
    assert!(invoke_random(Builtin::Chance, &[Value::Boolean(true)], &mut state, 1).is_err());
    assert!(invoke_random(Builtin::Len, &[Value::Integer(1)], &mut state, 1).is_err());
}
