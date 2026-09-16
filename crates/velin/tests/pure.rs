use std::{collections::BTreeMap, sync::Arc};

use velin::{ExecutionPolicy, PureModule, PureModuleError, Type, Value};

fn inputs(entries: impl IntoIterator<Item = (&'static str, Type)>) -> BTreeMap<String, Type> {
    entries
        .into_iter()
        .map(|(name, ty)| (name.to_owned(), ty))
        .collect()
}

#[test]
fn invokes_with_fresh_state_and_returns_value() {
    let module = PureModule::compile(
        "reward.velin",
        "set next = input + 1\nperform return(next)\n",
        inputs([("input", Type::Integer)]),
    )
    .unwrap();
    let call = |value| module.invoke(BTreeMap::from([("input".into(), Value::Integer(value))]));
    assert_eq!(call(41), Ok(Value::Integer(42)));
    assert_eq!(call(41), Ok(Value::Integer(42)));
}

#[test]
fn explicit_policy_applies_to_pure_invocations() {
    let module = PureModule::compile(
        "policy.velin",
        "perform return(input + 1)\n",
        inputs([("input", Type::Integer)]),
    )
    .unwrap();
    assert_eq!(
        module.invoke_with_policy(
            BTreeMap::from([("input".into(), Value::Integer(1))]),
            ExecutionPolicy::default().with_max_fuel(0),
        ),
        Err(PureModuleError::FuelExhausted)
    );
}

#[test]
fn validates_input_names_and_types() {
    let module = PureModule::compile(
        "input.velin",
        "perform return(input)\n",
        inputs([("input", Type::Integer)]),
    )
    .unwrap();
    assert_eq!(
        module.invoke(BTreeMap::new()),
        Err(PureModuleError::MissingInput("input".into()))
    );
    assert_eq!(
        module.invoke(BTreeMap::from([(
            "input".into(),
            Value::String("wrong".into()),
        )])),
        Err(PureModuleError::InputType {
            name: "input".into(),
            expected: Type::Integer,
            found: Type::String,
        })
    );
    assert_eq!(
        module.invoke(BTreeMap::from([
            ("input".into(), Value::Integer(1)),
            ("extra".into(), Value::Integer(2)),
        ])),
        Err(PureModuleError::UnknownInput("extra".into()))
    );
}

#[test]
fn handles_explicit_failure_and_missing_return() {
    let failing = PureModule::compile(
        "failure.velin",
        "perform fail(\"missing key\")\n",
        BTreeMap::new(),
    )
    .unwrap();
    assert_eq!(
        failing.invoke(BTreeMap::new()),
        Err(PureModuleError::ExplicitFailure("missing key".into()))
    );

    let missing = PureModule::compile("missing.velin", "set x = 1\n", BTreeMap::new()).unwrap();
    assert_eq!(
        missing.invoke(BTreeMap::new()),
        Err(PureModuleError::MissingReturn)
    );
}

#[test]
fn rejects_effects_and_randomness_at_compile_time() {
    let effect = PureModule::compile("effect.velin", "perform say(1)\n", BTreeMap::new());
    assert!(matches!(effect, Err(PureModuleError::Check(_))));

    let random = PureModule::compile(
        "random.velin",
        "set value = random(1, 6)\nperform return(value)\n",
        BTreeMap::new(),
    );
    assert!(
        matches!(random, Err(PureModuleError::Check(errors)) if errors.iter().any(|error| error.message.contains("random and chance")))
    );

    let malformed_return =
        PureModule::compile("return.velin", "perform return()\n", BTreeMap::new());
    assert!(matches!(malformed_return, Err(PureModuleError::Check(_))));

    let bound_return = PureModule::compile(
        "return.velin",
        "answer = perform return(1)\n",
        BTreeMap::new(),
    );
    assert!(matches!(bound_return, Err(PureModuleError::Check(_))));
}

#[test]
fn checks_input_bindings_with_defaults() {
    let module = PureModule::compile(
        "types.velin",
        "default prefix = \"x\"\nset output = input + prefix\nperform return(output)\n",
        inputs([("input", Type::String)]),
    )
    .unwrap();
    assert_eq!(
        module.invoke(BTreeMap::from([(
            "input".into(),
            Value::String("y".into())
        )])),
        Ok(Value::String("yx".into()))
    );
}

#[test]
fn dynamic_failure_values_are_checked_at_runtime() {
    let module = PureModule::compile(
        "failure.velin",
        "perform fail(input)\n",
        inputs([("input", Type::Unknown)]),
    )
    .unwrap();
    assert!(matches!(
        module.invoke(BTreeMap::from([("input".into(), Value::Integer(1))])),
        Err(PureModuleError::InvalidReturn(message)) if message.contains("fail expects a string")
    ));
}

#[test]
fn preserves_list_and_record_values() {
    let list_module = PureModule::compile(
        "list.velin",
        "perform return(push(input, 2))\n",
        inputs([("input", Type::List)]),
    )
    .unwrap();
    assert_eq!(
        list_module.invoke(BTreeMap::from([(
            "input".into(),
            Value::List(Arc::new(vec![Value::Integer(1)])),
        )])),
        Ok(Value::List(Arc::new(vec![
            Value::Integer(1),
            Value::Integer(2),
        ])))
    );

    let record_module = PureModule::compile(
        "record.velin",
        "perform return(put(input, \"b\", 2))\n",
        inputs([("input", Type::Record)]),
    )
    .unwrap();
    assert_eq!(
        record_module.invoke(BTreeMap::from([(
            "input".into(),
            Value::Record(Arc::new(BTreeMap::from([("a".into(), Value::Integer(1))]))),
        )])),
        Ok(Value::Record(Arc::new(BTreeMap::from([
            ("a".into(), Value::Integer(1)),
            ("b".into(), Value::Integer(2)),
        ]))))
    );
}

#[test]
fn reusable_invoker_supports_single_inputs_and_batch_calls() {
    let module = PureModule::compile(
        "invoker.velin",
        "perform return(value + 1)\n",
        inputs([("value", Type::Integer)]),
    )
    .unwrap();
    let mut invoker = module.invoker().unwrap();
    assert!(!invoker.machine().profile_enabled());
    assert_eq!(invoker.invoke_one(Value::Integer(4)), Ok(Value::Integer(5)));
    assert_eq!(invoker.invoke_one(Value::Integer(4)), Ok(Value::Integer(5)));
    let batch = invoker
        .invoke_batch([
            BTreeMap::from([("value".into(), Value::Integer(1))]),
            BTreeMap::from([("value".into(), Value::Integer(2))]),
        ])
        .unwrap();
    assert_eq!(batch, vec![Value::Integer(2), Value::Integer(3)]);
}

#[test]
fn single_input_fast_path_rejects_the_wrong_module_arity() {
    let module = PureModule::compile(
        "two-inputs.velin",
        "perform return(left + right)\n",
        inputs([("left", Type::Integer), ("right", Type::Integer)]),
    )
    .unwrap();
    let mut invoker = module.invoker().unwrap();
    assert_eq!(
        invoker.invoke_one(Value::Integer(1)),
        Err(PureModuleError::InvalidInput(
            "invoke_one requires exactly one declared input".into(),
        ))
    );
}

#[test]
fn length_guard_fast_path_preserves_loop_results() {
    let module = PureModule::compile(
        "sum.velin",
        "set total = 0\nset index = 0\nwhile index < len(values):\n    set total = total + get(values, index)\n    set index = index + 1\nperform return(total)\n",
        inputs([("values", Type::List)]),
    )
    .unwrap();
    let mut invoker = module.invoker().unwrap();
    let values = Value::List(Arc::new((0..256).map(Value::Integer).collect()));
    assert_eq!(invoker.invoke_one(values), Ok(Value::Integer(32_640)));
}
