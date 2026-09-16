use super::*;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use velin_compile::ProgramBuilder;
use velin_syntax::Expr;

#[test]
fn non_boolean_conditions_are_rejected() {
    let mut b = ProgramBuilder::new();
    let condition = b.expr(&Expr::Value(Value::Integer(1)), 4);
    b.push(Op::JumpIfFalse {
        condition,
        target: 1,
    });
    let error = Machine::new(b.build()).unwrap().run().unwrap_err();
    assert_eq!(error.line, 4);
    assert!(error.message.contains("condition expects boolean"));
}

#[test]
fn bound_host_effects_require_a_resume_value() {
    let mut b = ProgramBuilder::new();
    let choice = b.slot("choice");
    b.push(Op::host(1, Vec::new(), Some(choice), 7));
    let mut machine = Machine::new(b.build()).unwrap();
    machine.run().unwrap();

    let error = machine.resume(None).unwrap_err();
    assert_eq!(error.line, 7);
    assert!(error.message.contains("returned no value"));
    assert!(machine.variable("choice").is_none());

    let oversized = Value::String("x".repeat(velin_syntax::MAX_DATA_TEXT_BYTES + 1).into());
    let error = machine.resume(Some(oversized)).unwrap_err();
    assert_eq!(error.line, 7);
    assert!(error.message.contains("data text exceeds"));
    assert!(machine.variable("choice").is_none());

    assert_eq!(
        machine.resume(Some(Value::Integer(2))).unwrap(),
        Yield::Finished
    );
    assert_eq!(machine.variable("choice"), Some(&Value::Integer(2)));
}

#[test]
fn run_cannot_skip_a_pending_host_effect() {
    let mut b = ProgramBuilder::new();
    b.push(Op::host(1, Vec::new(), None, 3));
    let mut machine = Machine::new(b.build()).unwrap();
    machine.run().unwrap();
    assert!(machine.run().unwrap_err().message.contains("call `resume`"));
    assert_eq!(machine.resume(None).unwrap(), Yield::Finished);
    assert!(
        machine
            .resume(None)
            .unwrap_err()
            .message
            .contains("no host effect")
    );
}

#[test]
fn external_values_must_fit_the_data_budget() {
    let mut b = ProgramBuilder::new();
    b.slot("seed");
    let mut machine = Machine::new(b.build()).unwrap();
    let oversized = Value::String("x".repeat(velin_syntax::MAX_DATA_TEXT_BYTES + 1).into());
    assert!(!machine.set_variable("seed", oversized));
    assert!(machine.variable("seed").is_none());

    assert_eq!(
        machine
            .try_set_variable("missing", Value::Integer(1))
            .unwrap_err(),
        SetVariableError::UnknownVariable("missing".into())
    );
}

#[test]
fn execution_policy_accumulates_fuel_across_resumes_and_restart_clears_it() {
    let mut builder = ProgramBuilder::new();
    builder.push(Op::host(1, Vec::new(), None, 1));
    builder.push(Op::host(2, Vec::new(), None, 1));
    let policy = ExecutionPolicy::default().with_max_fuel(1);
    let mut machine = Machine::with_seed_and_policy(builder.build(), 0, policy).unwrap();
    assert!(matches!(
        machine.run().unwrap(),
        Yield::Host { host_id: 1, .. }
    ));
    assert_eq!(machine.fuel_used(), 1);
    let error = machine.resume(None).unwrap_err();
    assert_eq!(error.kind(), velin_eval::EvalErrorKind::FuelExhausted);
    assert_eq!(machine.fuel_used(), 1);

    let snapshot = machine.clone();
    assert_eq!(snapshot.fuel_used(), machine.fuel_used());
    let initial =
        InitialFrame::from_named_values(&machine.program().slots, std::iter::empty()).unwrap();
    machine.restart(&initial, 0).unwrap();
    assert_eq!(machine.fuel_used(), 0);
    assert_eq!(machine.host_effects(), 0);
}

#[test]
fn immediate_fuel_resets_for_resume_and_progress_callback_can_cancel() {
    let mut builder = ProgramBuilder::new();
    builder.push(Op::host(1, Vec::new(), None, 1));
    builder.push(Op::host(2, Vec::new(), None, 1));
    let mut machine = Machine::with_seed_and_policy(
        builder.build(),
        0,
        ExecutionPolicy::default().with_max_immediate_fuel(1),
    )
    .unwrap();
    machine.run().unwrap();
    assert!(matches!(
        machine.resume(None).unwrap(),
        Yield::Host { host_id: 2, .. }
    ));

    let calls = Arc::new(AtomicUsize::new(0));
    let callback_calls = Arc::clone(&calls);
    // A non-empty program is needed to cross a fuel checkpoint.
    let mut loop_builder = ProgramBuilder::new();
    loop_builder.push(Op::Jump(0));
    let mut cancelling = Machine::with_seed_and_policy(
        loop_builder.build(),
        0,
        ExecutionPolicy::default().with_progress_callback(1, move |_| {
            callback_calls.fetch_add(1, Ordering::Relaxed);
            false
        }),
    )
    .unwrap();
    assert_eq!(
        cancelling.run().unwrap_err().kind(),
        velin_eval::EvalErrorKind::Cancelled
    );
    assert!(calls.load(Ordering::Relaxed) > 0);
}

#[test]
fn custom_value_and_host_effect_budgets_are_enforced() {
    let mut builder = ProgramBuilder::new();
    let value = builder.slot("value");
    builder.push(Op::SetConst {
        slot: value,
        value: Value::String("ab".into()),
        line: 1,
    });
    let mut machine = Machine::with_seed_and_policy(
        builder.build(),
        0,
        ExecutionPolicy::default()
            .with_value_budgets(1, 1)
            .with_max_host_effects(1),
    )
    .unwrap();
    assert!(machine.run().is_err());

    let mut hosts = ProgramBuilder::new();
    hosts.push(Op::host(1, Vec::new(), None, 1));
    hosts.push(Op::host(2, Vec::new(), None, 1));
    let mut machine = Machine::with_seed_and_policy(
        hosts.build(),
        0,
        ExecutionPolicy::default().with_max_host_effects(1),
    )
    .unwrap();
    machine.run().unwrap();
    assert_eq!(
        machine.resume(None).unwrap_err().kind(),
        velin_eval::EvalErrorKind::HostEffectsExceeded
    );
}

#[test]
fn ownership_update_rejects_growth_without_changing_the_slot() {
    let mut builder = ProgramBuilder::new();
    let items = builder.slot("items");
    let value = builder.expr(&Expr::Value(Value::Integer(1)), 4);
    builder.push(Op::Update {
        slot: items,
        operation: UpdateOp::Push { value },
        line: 4,
        column: 1,
    });
    let mut machine = Machine::new(builder.build()).unwrap();
    let original = Value::List(Arc::new(vec![
        Value::Integer(0);
        velin_syntax::MAX_DATA_VALUES - 1
    ]));
    machine.try_set_variable("items", original.clone()).unwrap();
    let error = machine.run().unwrap_err();
    assert_eq!(
        error.message,
        "data exceeds 4096 values or 16 nesting levels"
    );
    assert_eq!(machine.variable("items"), Some(&original));
}

#[test]
fn cached_collection_depth_is_recomputed_after_removing_the_deepest_child() {
    let mut builder = ProgramBuilder::new();
    let items = builder.slot("items");
    let outer = builder.slot("outer");
    let key = builder.expr(&Expr::Value(Value::Integer(0)), 1);
    builder.push(Op::update(items, UpdateOp::Remove { key }, 1, 1));
    let value = builder.expr(&Expr::Variable("items".into()), 2);
    builder.push(Op::update(outer, UpdateOp::Push { value }, 2, 1));

    let mut deep = Value::Integer(1);
    for _ in 0..15 {
        deep = Value::List(Arc::new(vec![deep]));
    }
    let mut machine = Machine::new(builder.build()).unwrap();
    machine
        .try_set_variable("items", Value::List(Arc::new(vec![deep])))
        .unwrap();
    machine
        .try_set_variable("outer", Value::List(Arc::new(Vec::new())))
        .unwrap();
    assert_eq!(machine.run().unwrap(), Yield::Finished);
    assert_eq!(
        machine.variable("outer"),
        Some(&Value::List(Arc::new(vec![Value::List(Arc::new(
            Vec::new()
        ))])))
    );
}

#[test]
fn aggregate_machine_state_text_is_bounded_and_replacements_release_budget() {
    let mut builder = ProgramBuilder::new();
    let names: Vec<String> = (0..=16).map(|index| format!("value_{index}")).collect();
    for name in &names {
        builder.slot(name);
    }
    let mut machine = Machine::new(builder.build()).unwrap();
    let payload = Value::String("x".repeat(velin_syntax::MAX_DATA_TEXT_BYTES).into());

    for name in &names[..16] {
        machine.try_set_variable(name, payload.clone()).unwrap();
    }
    let error = machine
        .try_set_variable(&names[16], payload.clone())
        .unwrap_err();
    assert_eq!(
        error,
        SetVariableError::StateBudget("machine state text exceeds 16 MiB")
    );
    assert!(machine.variable(&names[16]).is_none());

    machine
        .try_set_variable(&names[0], Value::Integer(1))
        .unwrap();
    machine.try_set_variable(&names[16], payload).unwrap();
}

#[test]
fn script_assignments_and_host_payloads_have_aggregate_budgets() {
    let payload = Value::String("x".repeat(velin_syntax::MAX_DATA_TEXT_BYTES).into());

    let mut assignments = ProgramBuilder::new();
    let chunk = assignments.expr(&Expr::Value(payload.clone()), 4);
    let slots: Vec<u32> = (0..=16)
        .map(|index| assignments.slot(&format!("value_{index}")))
        .collect();
    for slot in &slots {
        assignments.push(Op::Set {
            slot: *slot,
            value: chunk,
        });
    }
    let mut machine = Machine::new(assignments.build()).unwrap();
    let error = machine.run().unwrap_err();
    assert_eq!(error.line, 4);
    assert_eq!(error.message, "machine state text exceeds 16 MiB");
    assert!(machine.variable("value_16").is_none());

    let mut host = ProgramBuilder::new();
    let argument = host.expr(&Expr::Value(payload), 7);
    host.push(Op::host(1, vec![argument; 17], None, 7));
    let error = Machine::new(host.build()).unwrap().run().unwrap_err();
    assert_eq!(error.line, 7);
    assert_eq!(error.message, "host payload text exceeds 16 MiB");
}

#[test]
fn rng_seed_and_unknown_variables_and_jump_off_end() {
    let mut plain = ProgramBuilder::new();
    plain.push(Op::Halt);
    let mut machine = Machine::new(plain.build()).unwrap();
    assert!(!machine.set_rng_seed(1));
    assert!(machine.rng_state().is_none());
    assert!(!machine.set_variable("missing", Value::Integer(1)));

    let mut random = ProgramBuilder::new();
    let roll = random.slot("roll");
    let expression = velin_parse::parse_expression("random(0, 10)", "t", 1, 1).unwrap();
    let chunk = random.expr(&expression, 1);
    random.push(Op::Set {
        slot: roll,
        value: chunk,
    });
    let mut machine = Machine::with_seed(random.build(), 1).unwrap();
    assert!(machine.set_rng_seed(99));
    assert_eq!(machine.rng_state(), Some(99));
    machine.set_variable(velin_compile::RNG_STATE_SLOT, Value::Boolean(true));
    assert!(machine.rng_state().is_none());

    let mut jump = ProgramBuilder::new();
    jump.push(Op::Jump(8));
    let error = Machine::new(jump.build()).unwrap_err();
    assert!(error.message.contains("past program end"));
}

#[test]
fn reseeding_rng_refreshes_frame_metrics() {
    let mut builder = ProgramBuilder::new();
    let roll = builder.slot("roll");
    let expression = velin_parse::parse_expression("random(0, 10)", "t", 1, 1).unwrap();
    let chunk = builder.expr(&expression, 1);
    builder.push(Op::Set {
        slot: roll,
        value: chunk,
    });
    let names: Vec<String> = (0..16).map(|index| format!("value_{index}")).collect();
    for name in &names {
        builder.slot(name);
    }

    let mut machine = Machine::new(builder.build()).unwrap();
    let payload = Value::String("x".repeat(velin_syntax::MAX_DATA_TEXT_BYTES).into());
    machine
        .try_set_variable(velin_compile::RNG_STATE_SLOT, payload.clone())
        .unwrap();
    assert!(machine.set_rng_seed(1));

    for name in &names {
        machine.try_set_variable(name, payload.clone()).unwrap();
    }
    assert_eq!(
        machine.frame_total.text_bytes,
        16 * velin_syntax::MAX_DATA_TEXT_BYTES
    );
}
