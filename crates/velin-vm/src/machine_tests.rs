use super::*;
use velin_compile::ProgramBuilder;
use velin_syntax::{BinaryOp, Expr};

#[test]
fn runs_assignments_and_conditionals_to_completion() {
    // hp = 30; if hp < 20 { hp = 0 }  -> hp stays 30
    let mut b = ProgramBuilder::new();
    let hp = b.slot("hp");
    let thirty = b.expr(&Expr::Value(Value::Integer(30)), 1);
    b.push(Op::Set {
        slot: hp,
        value: thirty,
    });
    let cond = b.expr(
        &Expr::Binary {
            left: Box::new(Expr::Variable("hp".into())),
            op: BinaryOp::Less,
            right: Box::new(Expr::Value(Value::Integer(20))),
        },
        2,
    );
    let jump = b.push(Op::JumpIfFalse {
        condition: cond,
        target: u32::MAX,
    });
    let zero = b.expr(&Expr::Value(Value::Integer(0)), 3);
    b.push(Op::Set {
        slot: hp,
        value: zero,
    });
    let end = b.here();
    b.patch(
        jump,
        Op::JumpIfFalse {
            condition: cond,
            target: end,
        },
    );
    let mut machine = Machine::new(b.build()).unwrap();
    assert_eq!(machine.run().unwrap(), Yield::Finished);
    assert_eq!(machine.variable("hp"), Some(&Value::Integer(30)));
}

#[test]
fn direct_assignment_and_integer_guard_ops_preserve_semantics() {
    let mut builder = ProgramBuilder::new();
    let source = builder.slot("source");
    let copy = builder.slot("copy");
    let set = builder.set_op(source, &Expr::Value(Value::Integer(7)), 1);
    builder.push(set);
    let copy_op = builder.set_op(copy, &Expr::Variable("source".into()), 2);
    builder.push(copy_op);
    let comparison = Expr::Binary {
        left: Box::new(Expr::Variable("copy".into())),
        op: BinaryOp::GreaterEqual,
        right: Box::new(Expr::Value(Value::Integer(7))),
    };
    let guard = builder.jump_if_false_op(&comparison, 3, 5);
    builder.push(guard);
    let set = builder.set_op(copy, &Expr::Value(Value::Integer(9)), 4);
    builder.push(set);

    let mut machine = Machine::new(builder.build()).unwrap();
    assert_eq!(machine.run().unwrap(), Yield::Finished);
    assert_eq!(machine.variable("copy"), Some(&Value::Integer(9)));
}

#[test]
fn quickened_readonly_builtins_preserve_results_and_errors() {
    let mut builder = ProgramBuilder::new();
    builder.slot("bag");
    for (target, source) in [
        ("size", "len(bag)"),
        ("present", "contains(bag, \"map\")"),
        ("first", "get(bag, 0)"),
        ("fallback", "get(bag, 9, \"missing\")"),
    ] {
        let target = builder.slot(target);
        let expression = velin_parse::parse_expression(source, "quickened", 4, 1).unwrap();
        let operation = builder.set_op(target, &expression, 4);
        builder.push(operation);
    }
    let program = builder.build();

    let mut unassigned = Machine::new(program.clone()).unwrap();
    let error = unassigned.run().unwrap_err();
    assert_eq!(error.line, 4);
    assert!(error.message.contains("`bag` has not been assigned"));

    let mut wrong_type = Machine::new(program.clone()).unwrap();
    wrong_type.set_variable("bag", Value::Integer(1));
    assert_eq!(
        wrong_type.run().unwrap_err().message,
        "len expects a list, record or string"
    );

    let mut machine = Machine::new(program).unwrap();
    machine.set_variable(
        "bag",
        Value::List(Arc::new(vec![Value::String("map".into())])),
    );
    assert_eq!(machine.run().unwrap(), Yield::Finished);
    assert_eq!(machine.variable("size"), Some(&Value::Integer(1)));
    assert_eq!(machine.variable("present"), Some(&Value::Boolean(true)));
    assert_eq!(
        machine.variable("first"),
        Some(&Value::String("map".into()))
    );
    assert_eq!(
        machine.variable("fallback"),
        Some(&Value::String("missing".into()))
    );
}

#[test]
fn specialized_integer_guard_keeps_type_and_assignment_errors() {
    let mut builder = ProgramBuilder::new();
    builder.slot("source");
    let comparison = Expr::Binary {
        left: Box::new(Expr::Variable("source".into())),
        op: BinaryOp::Less,
        right: Box::new(Expr::Value(Value::Integer(7))),
    };
    let guard = builder.jump_if_false_op(&comparison, 8, 1);
    builder.push(guard);
    let program = builder.build();

    let mut unassigned = Machine::new(program.clone()).unwrap();
    let error = unassigned.run().unwrap_err();
    assert_eq!(error.line, 8);
    assert!(error.message.contains("has not been assigned"));

    let mut wrong_type = Machine::new(program).unwrap();
    wrong_type
        .try_set_variable("source", Value::Boolean(true))
        .unwrap();
    let error = wrong_type.run().unwrap_err();
    assert_eq!(error.line, 8);
    assert_eq!(
        error.message,
        "comparison cannot combine boolean and integer"
    );
}

#[test]
fn yields_host_effects_and_binds_resume_values() {
    // ask(host 1); set choice from resume; done
    let mut b = ProgramBuilder::new();
    let choice = b.slot("choice");
    let prompt = b.expr(&Expr::Value(Value::String("pick".into())), 1);
    b.push(Op::host(1, vec![prompt], Some(choice), 1));
    let mut machine = Machine::new(b.build()).unwrap();
    let effect = machine.run().unwrap();
    assert_eq!(
        effect,
        Yield::Host {
            host_id: 1,
            values: vec![Value::String("pick".into())]
        }
    );
    assert_eq!(
        machine.resume(Some(Value::Integer(2))).unwrap(),
        Yield::Finished
    );
    assert_eq!(machine.variable("choice"), Some(&Value::Integer(2)));
}

#[test]
fn infinite_loops_are_bounded() {
    // loop: jump loop
    let mut b = ProgramBuilder::new();
    b.push(Op::Jump(0));
    let mut machine = Machine::new(b.build()).unwrap();
    let error = machine.run().unwrap_err();
    assert!(error.message.contains("infinite loop"));
}

#[test]
fn cloning_a_machine_rolls_back_rng_with_the_frame() {
    // Draw and yield once, clone the yielded machine as a checkpoint, then
    // draw again. Resuming the checkpoint must reproduce the second draw.
    let mut b = ProgramBuilder::new();
    let first = b.slot("first");
    let second = b.slot("second");
    let first_roll = velin_parse::parse_expression("random(1, 1000)", "t", 1, 1).unwrap();
    let first_chunk = b.expr(&first_roll, 1);
    b.push(Op::Set {
        slot: first,
        value: first_chunk,
    });
    let show_first = b.expr(&Expr::Variable("first".into()), 2);
    b.push(Op::host(1, vec![show_first], None, 2));
    let second_roll = velin_parse::parse_expression("random(1, 1000)", "t", 3, 1).unwrap();
    let second_chunk = b.expr(&second_roll, 3);
    b.push(Op::Set {
        slot: second,
        value: second_chunk,
    });
    let show_second = b.expr(&Expr::Variable("second".into()), 4);
    b.push(Op::host(2, vec![show_second], None, 4));

    let mut machine = Machine::with_seed(b.build(), 42).unwrap();
    assert!(matches!(
        machine.run().unwrap(),
        Yield::Host { host_id: 1, .. }
    ));
    let checkpoint = machine.clone();

    let expected = machine.resume(None).unwrap();
    let expected_state = machine.rng_state();
    let mut restored = checkpoint;
    let replayed = restored.resume(None).unwrap();
    assert_eq!(replayed, expected);
    assert_eq!(restored.rng_state(), expected_state);
    assert_eq!(restored.variable("second"), machine.variable("second"));
}

#[test]
fn machine_snapshots_keep_independent_frames_after_writes() {
    let mut builder = ProgramBuilder::new();
    builder.slot("value");
    let mut machine = Machine::new(builder.build()).unwrap();
    machine
        .try_set_variable("value", Value::String("before".into()))
        .unwrap();
    let snapshot = machine.clone();

    machine
        .try_set_variable("value", Value::String("after".into()))
        .unwrap();
    assert_eq!(
        snapshot.variable("value"),
        Some(&Value::String("before".into()))
    );
    assert_eq!(
        machine.variable("value"),
        Some(&Value::String("after".into()))
    );
}

#[test]
fn the_same_seed_replays_and_different_seeds_diverge() {
    let mut b = ProgramBuilder::new();
    let roll = b.slot("roll");
    let expression = velin_parse::parse_expression("random(0, 1000000)", "t", 1, 1).unwrap();
    let chunk = b.expr(&expression, 1);
    b.push(Op::Set {
        slot: roll,
        value: chunk,
    });
    let program = b.build();

    let mut first = Machine::with_seed(program.clone(), 5).unwrap();
    let mut replay = Machine::with_seed(program.clone(), 5).unwrap();
    let mut different = Machine::with_seed(program, 6).unwrap();
    first.run().unwrap();
    replay.run().unwrap();
    different.run().unwrap();
    assert_eq!(first.variable("roll"), replay.variable("roll"));
    assert_eq!(first.rng_state(), replay.rng_state());
    assert_ne!(first.rng_state(), different.rng_state());
}

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
