use super::*;
use crate::{HostSchema, HostSignature, Type, compile};
use std::error::Error;

#[test]
fn queue_defaults_and_error_display_cover_every_variant() {
    let limits = HostEventQueueLimits::default();
    let queue = HostEventQueue::new(limits);
    assert_eq!(queue.limits(), limits);
    assert!(queue.is_empty());
    assert_eq!(
        HostEventQueueError::Full.to_string(),
        "host event queue is full"
    );
    assert_eq!(
        HostEventQueueError::ValuesBudget.to_string(),
        "host event queue value budget exceeded"
    );
    assert_eq!(
        HostEventQueueError::TextBudget.to_string(),
        "host event queue text budget exceeded"
    );
    assert_eq!(HostEventQueueError::InvalidValue("bad").to_string(), "bad");
    assert!(Error::source(&HostEventQueueError::Full).is_none());

    let mut tiny = HostEventQueue::new(HostEventQueueLimits {
        capacity: 2,
        max_values: 1,
        max_text_bytes: 8,
    });
    assert_eq!(
        tiny.push(HostEvent {
            name: "emit".into(),
            values: vec![Value::Integer(1), Value::Integer(2)],
        }),
        Err(HostEventQueueError::ValuesBudget)
    );
    let huge = Value::String("x".repeat(velin_syntax::MAX_DATA_TEXT_BYTES + 1).into());
    assert!(matches!(
        tiny.push(HostEvent {
            name: "emit".into(),
            values: vec![huge],
        }),
        Err(HostEventQueueError::InvalidValue(_))
    ));
    assert!(tiny.pop_front().is_none());
}

#[test]
fn script_run_errors_display_and_expose_sources() {
    let mut script = compile("runner.velin", "set value = 1\n").unwrap();
    script.program = std::sync::Arc::new(crate::Program {
        ops: vec![crate::Op::Jump(2)],
        chunks: Vec::new(),
        expr_ops: Vec::new(),
        constants: Vec::new(),
        slots: crate::SlotTable::new(),
    });
    let Err(program) = ScriptRunner::new(&script) else {
        panic!("expected invalid bytecode");
    };
    assert!(program.to_string().contains("invalid bytecode"));
    assert!(Error::source(&program).is_some());

    let mut script = compile("runner.velin", "default hp = 3\n").unwrap();
    script.defaults.insert("unknown".into(), Value::Integer(1));
    let Err(initial) = ScriptRunner::new(&script) else {
        panic!("expected unknown default");
    };
    assert!(initial.to_string().contains("cannot initialize"));
    assert!(Error::source(&initial).is_some());

    let script = compile("runner.velin", "set value = 1 / 0\n").unwrap();
    let mut runner = ScriptRunner::new(&script).unwrap();
    let evaluation = runner.run().unwrap_err();
    assert!(evaluation.to_string().contains("runtime error"));
    assert!(Error::source(&evaluation).is_some());

    let script = compile("runner.velin", "perform emit(1)\n").unwrap();
    let mut runner = ScriptRunner::configured(
        &script,
        0,
        ExecutionLimits {
            max_host_effects: 0,
        },
        None,
    )
    .unwrap();
    let budget = runner.run().unwrap_err();
    assert!(budget.to_string().contains("too many host effects"));
    assert!(Error::source(&budget).is_none());
    assert!(matches!(
        runner.run().unwrap_err(),
        ScriptRunError::HostEffectsExceeded { limit: 0 }
    ));

    let mut script = compile("runner.velin", "perform emit(1)\n").unwrap();
    script.hosts.clear();
    let mut runner = ScriptRunner::new(&script).unwrap();
    assert!(matches!(
        runner.run().unwrap_err(),
        ScriptRunError::HostContract(_)
    ));
    assert!(matches!(
        runner.run().unwrap_err(),
        ScriptRunError::HostContract(_)
    ));
    let mut runner = ScriptRunner::new(&script).unwrap();
    assert!(matches!(
        runner.run_effect_batch(4).unwrap_err(),
        ScriptRunError::HostContract(_)
    ));
}

#[test]
fn runner_covers_restart_batch_and_schema_contract_paths() {
    let mut script = compile("runner.velin", "default hp = 3\nperform emit(hp)\n").unwrap();
    script.defaults.insert("hp".into(), Value::Integer(9));
    assert!(script.initial_frame().is_none());
    let mut runner = ScriptRunner::new(&script).unwrap();
    assert_eq!(runner.machine().variable("hp"), Some(&Value::Integer(9)));
    runner.restart(1).unwrap();
    assert_eq!(runner.host_effects(), 0);
    assert!(
        runner
            .try_set_variable("missing", Value::Integer(1))
            .is_err()
    );

    let script = compile("runner.velin", "perform emit(1)\nperform emit(2)\n").unwrap();
    let mut runner = ScriptRunner::configured(
        &script,
        0,
        ExecutionLimits {
            max_host_effects: 0,
        },
        None,
    )
    .unwrap();
    assert!(matches!(
        runner.run_effect_batch(4).unwrap_err(),
        ScriptRunError::HostEffectsExceeded { limit: 0 }
    ));

    let script = compile("runner.velin", "perform emit(1)\n").unwrap();
    let schema = HostSchema::new().command("other", HostSignature::exact(Vec::new(), None));
    let mut runner =
        ScriptRunner::configured(&script, 0, ExecutionLimits::default(), Some(&schema)).unwrap();
    assert!(matches!(
        runner.run_effect_batch(4).unwrap_err(),
        ScriptRunError::HostContract(_)
    ));
    assert!(matches!(
        runner.run_effect_batch(4).unwrap_err(),
        ScriptRunError::HostContract(_)
    ));

    let schema = HostSchema::new()
        .allow_unknown(true)
        .command("emit", HostSignature::exact(vec![Type::Integer], None));
    assert!(schema.allows_unknown());
    let mut runner =
        ScriptRunner::configured(&script, 0, ExecutionLimits::default(), Some(&schema)).unwrap();
    assert_eq!(runner.run_effect_batch(4).unwrap().len(), 1);

    let schema = HostSchema::new().command(
        "emit",
        HostSignature::exact(vec![Type::Integer, Type::Integer], None),
    );
    let mut runner =
        ScriptRunner::configured(&script, 0, ExecutionLimits::default(), Some(&schema)).unwrap();
    assert!(runner.run().unwrap_err().to_string().contains("argument"));

    let ask = compile("runner.velin", "answer = perform ask()\n").unwrap();
    let schema = HostSchema::new().command("ask", HostSignature::exact(Vec::new(), None));
    let mut runner =
        ScriptRunner::configured(&ask, 0, ExecutionLimits::default(), Some(&schema)).unwrap();
    runner.run().unwrap();
    assert!(matches!(
        runner.resume(Some(Value::Integer(1))).unwrap_err(),
        ScriptRunError::HostContract(_)
    ));
}
