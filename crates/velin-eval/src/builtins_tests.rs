use super::*;

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
    let mut stack = vec![Value::Integer(1), Value::String("xy".into())];
    let mut metrics = stack
        .iter()
        .map(|value| value.data_metrics().unwrap())
        .collect();
    let list_metrics =
        invoke_stack_measured_with_metrics(Builtin::List, &mut stack, &mut metrics, 2, 1).unwrap();
    assert_eq!(list_metrics, stack[0].data_metrics().unwrap());
    assert_eq!(metrics, vec![list_metrics]);

    let child = Value::List(Arc::new(vec![Value::Boolean(true)]));
    let child_metrics = child.data_metrics().unwrap();
    stack.push(child);
    metrics.push(child_metrics);
    let pushed_metrics =
        invoke_stack_measured_with_metrics(Builtin::Push, &mut stack, &mut metrics, 2, 1).unwrap();
    assert_eq!(pushed_metrics, stack[0].data_metrics().unwrap());
    assert_eq!(metrics, vec![pushed_metrics]);
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
