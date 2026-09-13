use super::*;
use crate::{HostSignature, Op, Program, SlotTable, compile};
use std::sync::Arc;

#[test]
fn initializes_defaults_resolves_names_and_counts_effects() {
    let script = compile(
        "runner.velin",
        "default hp = 3\nperform emit(hp)\nperform emit(hp)\n",
    )
    .unwrap();
    let mut runner = ScriptRunner::configured(
        &script,
        1,
        ExecutionLimits {
            max_host_effects: 1,
        },
        None,
    )
    .unwrap();
    assert_eq!(runner.machine().variable("hp"), Some(&Value::Integer(3)));
    assert!(matches!(
        runner.run().unwrap(),
        ScriptYield::Host { ref name, .. } if name == "emit"
    ));
    assert!(matches!(
        runner.resume(None).unwrap_err(),
        ScriptRunError::HostEffectsExceeded { limit: 1 }
    ));
    assert!(matches!(
        runner.resume(None).unwrap_err(),
        ScriptRunError::HostEffectsExceeded { limit: 1 }
    ));
}

#[test]
fn enforces_host_arguments_and_allows_reply_retry() {
    let emit = compile("runner.velin", "perform emit(1)\n").unwrap();
    let schema = HostSchema::new().command("emit", HostSignature::exact(vec![Type::String], None));
    let mut runner =
        ScriptRunner::configured(&emit, 0, ExecutionLimits::default(), Some(&schema)).unwrap();
    assert!(matches!(
        runner.run().unwrap_err(),
        ScriptRunError::HostContract(_)
    ));
    assert!(matches!(
        runner.resume(None).unwrap_err(),
        ScriptRunError::HostContract(_)
    ));

    let ask = compile("runner.velin", "answer = perform ask()\n").unwrap();
    let schema =
        HostSchema::new().command("ask", HostSignature::exact(Vec::new(), Some(Type::Integer)));
    let mut runner =
        ScriptRunner::configured(&ask, 0, ExecutionLimits::default(), Some(&schema)).unwrap();
    runner.run().unwrap();
    assert!(matches!(
        runner
            .resume(Some(Value::String("wrong".into())))
            .unwrap_err(),
        ScriptRunError::HostContract(_)
    ));
    assert_eq!(
        runner.resume(Some(Value::Integer(7))).unwrap(),
        ScriptYield::Finished
    );
    assert_eq!(
        runner.machine().variable("answer"),
        Some(&Value::Integer(7))
    );
}

#[test]
fn replacing_the_public_program_invalidates_the_cached_proof() {
    let mut script = compile("runner.velin", "set value = 1\n").unwrap();
    script.program = Arc::new(Program {
        ops: vec![Op::Jump(2)],
        chunks: Vec::new(),
        expr_ops: Vec::new(),
        constants: Vec::new(),
        slots: SlotTable::new(),
    });
    assert!(matches!(
        ScriptRunner::new(&script),
        Err(ScriptRunError::Program(_))
    ));
}

#[test]
fn changing_public_defaults_invalidates_the_prepared_frame() {
    let mut script = compile("runner.velin", "default hp = 3\n").unwrap();
    assert!(script.initial_frame().is_some());

    script.defaults.insert("hp".into(), Value::Integer(9));
    assert!(script.initial_frame().is_none());
    let runner = ScriptRunner::new(&script).unwrap();
    assert_eq!(runner.machine().variable("hp"), Some(&Value::Integer(9)));

    script.defaults.insert("unknown".into(), Value::Integer(1));
    assert!(matches!(
        ScriptRunner::new(&script),
        Err(ScriptRunError::InitialValue { ref name, .. }) if name == "unknown"
    ));
}

#[test]
fn restart_matches_a_new_runner_after_host_yield() {
    let script = compile(
        "runner.velin",
        "default hp = 3\nperform emit(hp)\nset hp = hp + 1\n",
    )
    .unwrap();
    let mut reused = ScriptRunner::new(&script).unwrap();
    let mut fresh = ScriptRunner::new(&script).unwrap();
    let first_yield = reused.run().unwrap();
    assert_eq!(first_yield, fresh.run().unwrap());
    assert_eq!(reused.host_effects(), 1);
    reused.restart(0).unwrap();
    assert_eq!(reused.host_effects(), 0);
    assert_eq!(reused.run().unwrap(), first_yield);
    assert_eq!(reused.resume(None).unwrap(), fresh.resume(None).unwrap());
    assert_eq!(
        reused.machine().variable("hp"),
        fresh.machine().variable("hp")
    );
}

#[test]
fn batches_unbound_effects_and_stops_at_bound_effect() {
    let script = compile(
        "runner.velin",
        "perform emit(1)\nperform emit(2)\nanswer = perform ask()\n",
    )
    .unwrap();
    let mut runner = ScriptRunner::new(&script).unwrap();
    assert_eq!(
        runner.run_effect_batch(8).unwrap(),
        vec![
            HostEvent {
                name: "emit".into(),
                values: vec![Value::Integer(1)],
            },
            HostEvent {
                name: "emit".into(),
                values: vec![Value::Integer(2)],
            },
        ]
    );
    assert!(matches!(
        runner.run().unwrap(),
        ScriptYield::Host { ref name, .. } if name == "ask"
    ));
    assert_eq!(
        runner.resume(Some(Value::Integer(9))).unwrap(),
        ScriptYield::Finished
    );
}

#[test]
fn batch_returns_valid_prefix_before_host_contract_failure() {
    let script = compile("runner.velin", "perform emit(1)\nperform other(2)\n").unwrap();
    let schema = HostSchema::new().command("emit", HostSignature::exact(vec![Type::Integer], None));
    let mut runner =
        ScriptRunner::configured(&script, 0, ExecutionLimits::default(), Some(&schema)).unwrap();

    let events = runner.run_effect_batch(8).unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].name, "emit");
    assert_eq!(runner.host_effects(), 1);
    assert!(matches!(
        runner.run_effect_batch(8),
        Err(ScriptRunError::HostContract(_))
    ));
}

#[test]
fn host_event_queue_applies_capacity_and_payload_backpressure() {
    let mut queue = HostEventQueue::new(HostEventQueueLimits {
        capacity: 1,
        max_values: 2,
        max_text_bytes: 4,
    });
    queue
        .push(HostEvent {
            name: "emit".into(),
            values: vec![Value::String("abc".into())],
        })
        .unwrap();
    assert_eq!(queue.len(), 1);
    assert_eq!(
        queue.push(HostEvent {
            name: "emit".into(),
            values: vec![Value::Integer(1)],
        }),
        Err(HostEventQueueError::Full)
    );
    let event = queue.pop_front().unwrap();
    assert_eq!(event.name, "emit");
    assert!(queue.is_empty());
    assert_eq!(queue.values(), 0);
    assert_eq!(queue.text_bytes(), 0);
    assert_eq!(
        queue.push(HostEvent {
            name: "emit".into(),
            values: vec![Value::String("abcde".into())],
        }),
        Err(HostEventQueueError::TextBudget)
    );
}
