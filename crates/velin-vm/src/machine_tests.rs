use super::*;
use velin_bytecode::ExprOp;
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
fn direct_boolean_guard_preserves_assignment_and_type_errors() {
    let mut builder = ProgramBuilder::new();
    let enabled = builder.slot("enabled");
    let condition = builder.expr(&Expr::Variable("enabled".into()), 6);
    let jump = builder.push(Op::JumpIfFalse {
        condition,
        target: u32::MAX,
    });
    builder.push(Op::Halt);
    builder.patch(
        jump,
        Op::JumpIfFalse {
            condition,
            target: 1,
        },
    );
    let program = builder.build();

    let mut unassigned = Machine::new(program.clone()).unwrap();
    assert!(matches!(
        unassigned.program.chunk(condition).unwrap().ops,
        [ExprOp::Load { dst: 0, slot, .. }] if *slot == enabled
    ));
    let error = unassigned.run().unwrap_err();
    assert_eq!(error.line, 6);
    assert!(error.message.contains("has not been assigned"));

    let mut wrong_type = Machine::new(program).unwrap();
    wrong_type.set_variable("enabled", Value::Integer(1));
    let error = wrong_type.run().unwrap_err();
    assert_eq!(error.line, 6);
    assert!(error.message.contains("condition expects boolean"));
}

#[test]
fn register_builtins_preserve_results_and_errors() {
    let mut builder = ProgramBuilder::new();
    builder.slot("bag");
    for (target, source) in [
        ("size", "len(bag)"),
        ("present", "contains(bag, \"map\")"),
        ("first", "get(bag, 0)"),
        ("fallback", "get(bag, 9, \"missing\")"),
    ] {
        let target = builder.slot(target);
        let expression = velin_parse::parse_expression(source, "register", 4, 1).unwrap();
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
fn fused_update_jump_keeps_the_original_step_budget() {
    let mut builder = ProgramBuilder::new();
    let index = builder.slot("index");
    builder.push(Op::update(index, UpdateOp::AddInteger { value: 1 }, 1, 1));
    builder.push(Op::Jump(0));
    let mut machine = Machine::new(builder.build()).unwrap();
    machine.set_variable("index", Value::Integer(0));

    let error = machine.run().unwrap_err();
    assert!(error.message.contains("infinite loop"));
    assert_eq!(machine.variable("index"), Some(&Value::Integer(5_000)));
}

#[test]
fn scalar_update_reuses_the_existing_frame_footprint() {
    let mut builder = ProgramBuilder::new();
    let counter = builder.slot("counter");
    builder.push(Op::SetConst {
        slot: counter,
        value: Value::Integer(1),
        line: 1,
    });
    builder.push(Op::update(counter, UpdateOp::AddInteger { value: 1 }, 1, 1));
    builder.push(Op::Halt);

    let mut machine = Machine::new(builder.build()).unwrap();
    machine.step().unwrap();
    let before = machine.frame_total;
    machine.step().unwrap();
    assert_eq!(machine.frame_total, before);
    assert_eq!(machine.variable("counter"), Some(&Value::Integer(2)));
}

#[test]
fn execution_profile_records_anonymous_op_hits_and_merges() {
    let mut builder = ProgramBuilder::new();
    let value = builder.slot("value");
    let constant = builder.expr(&Expr::Value(Value::Integer(1)), 1);
    builder.push(Op::Set {
        slot: value,
        value: constant,
    });
    builder.push(Op::Halt);
    let program = builder.build();
    let mut machine = Machine::new(program).unwrap();
    assert_eq!(machine.run(), Ok(Yield::Finished));
    let profile = machine.profile().clone();
    assert!(profile.op_hits().iter().any(|hits| *hits > 0));
    assert!(!profile.hot_ops(1).is_empty());
    let mut merged = profile.clone();
    merged.merge(&profile).unwrap();
    assert!(
        merged
            .op_hits()
            .iter()
            .zip(profile.op_hits())
            .all(|(merged, original)| *merged >= *original)
    );
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
