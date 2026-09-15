use super::*;
use velin_bytecode::{InitialFrame, UpdateOp, ValidatedProgram};
use velin_compile::ProgramBuilder;
use velin_syntax::{BinaryOp, Expr};

fn run(source_ops: impl FnOnce(&mut ProgramBuilder)) -> Machine {
    let mut builder = ProgramBuilder::new();
    source_ops(&mut builder);
    Machine::new(builder.build()).expect("valid program")
}

#[test]
fn string_and_type_error_updates_use_the_generic_add_path() {
    let mut machine = run(|builder| {
        let name = builder.slot("name");
        builder.push(Op::SetConst {
            slot: name,
            value: Value::String("a".into()),
            line: 1,
        });
        let rhs = builder.expr(&Expr::Value(Value::String("b".into())), 2);
        builder.push(Op::update(name, UpdateOp::Add { rhs }, 2, 1));
    });
    assert_eq!(machine.run().unwrap(), Yield::Finished);
    assert_eq!(machine.variable("name"), Some(&Value::String("ab".into())));
    let _ = machine.program();

    let mut machine = run(|builder| {
        let name = builder.slot("name");
        builder.push(Op::SetConst {
            slot: name,
            value: Value::Integer(1),
            line: 1,
        });
        let rhs = builder.expr(&Expr::Value(Value::String("b".into())), 2);
        builder.push(Op::update(name, UpdateOp::Add { rhs }, 2, 1));
    });
    assert!(
        machine
            .run()
            .unwrap_err()
            .message
            .contains("`+` cannot combine")
    );
}

#[test]
fn collection_updates_cover_push_put_remove_and_type_errors() {
    let mut machine = run(|builder| {
        let items = builder.slot("items");
        builder.push(Op::SetConst {
            slot: items,
            value: Value::List(std::sync::Arc::new(vec![Value::Integer(1)])),
            line: 1,
        });
        let value = builder.expr(&Expr::Value(Value::Integer(2)), 2);
        builder.push(Op::update(items, UpdateOp::Push { value }, 2, 1));
        let key = builder.expr(&Expr::Value(Value::Integer(0)), 3);
        let put = builder.expr(&Expr::Value(Value::Integer(9)), 3);
        builder.push(Op::update(items, UpdateOp::Put { key, value: put }, 3, 1));
        let remove = builder.expr(&Expr::Value(Value::Integer(1)), 4);
        builder.push(Op::update(items, UpdateOp::Remove { key: remove }, 4, 1));
    });
    assert_eq!(machine.run().unwrap(), Yield::Finished);
    assert_eq!(
        machine.variable("items"),
        Some(&Value::List(std::sync::Arc::new(vec![Value::Integer(9)])))
    );

    let mut machine = run(|builder| {
        let rec = builder.slot("rec");
        builder.push(Op::SetConst {
            slot: rec,
            value: Value::Record(std::sync::Arc::new(std::collections::BTreeMap::from([(
                "a".into(),
                Value::Integer(1),
            )]))),
            line: 1,
        });
        let key = builder.expr(&Expr::Value(Value::String("b".into())), 2);
        let value = builder.expr(&Expr::Value(Value::Integer(2)), 2);
        builder.push(Op::update(rec, UpdateOp::Put { key, value }, 2, 1));
        let remove = builder.expr(&Expr::Value(Value::String("a".into())), 3);
        builder.push(Op::update(rec, UpdateOp::Remove { key: remove }, 3, 1));
    });
    assert_eq!(machine.run().unwrap(), Yield::Finished);

    let mut machine = run(|builder| {
        let value = builder.slot("value");
        builder.push(Op::SetConst {
            slot: value,
            value: Value::Integer(1),
            line: 1,
        });
        let pushed = builder.expr(&Expr::Value(Value::Integer(2)), 2);
        builder.push(Op::update(value, UpdateOp::Push { value: pushed }, 2, 1));
    });
    assert!(
        machine
            .run()
            .unwrap_err()
            .message
            .contains("push expects a list")
    );

    let mut machine = run(|builder| {
        let value = builder.slot("value");
        builder.push(Op::SetConst {
            slot: value,
            value: Value::Integer(1),
            line: 1,
        });
        let key = builder.expr(&Expr::Value(Value::Integer(0)), 2);
        let put = builder.expr(&Expr::Value(Value::Integer(1)), 2);
        builder.push(Op::update(value, UpdateOp::Put { key, value: put }, 2, 1));
    });
    assert!(machine.run().unwrap_err().message.contains("put/remove"));

    let mut machine = run(|builder| {
        let value = builder.slot("value");
        builder.push(Op::SetConst {
            slot: value,
            value: Value::Integer(1),
            line: 1,
        });
        let key = builder.expr(&Expr::Value(Value::Integer(0)), 2);
        builder.push(Op::update(value, UpdateOp::Remove { key }, 2, 1));
    });
    assert!(machine.run().unwrap_err().message.contains("put/remove"));
}

#[test]
fn integer_update_overflow_and_non_integer_sources_are_reported() {
    let mut machine = run(|builder| {
        let counter = builder.slot("counter");
        builder.push(Op::SetConst {
            slot: counter,
            value: Value::Integer(i64::MAX),
            line: 1,
        });
        builder.push(Op::update(counter, UpdateOp::AddInteger { value: 1 }, 2, 1));
    });
    assert_eq!(machine.run().unwrap_err().message, "integer overflow");

    let mut machine = run(|builder| {
        let counter = builder.slot("counter");
        builder.push(Op::SetConst {
            slot: counter,
            value: Value::String("x".into()),
            line: 1,
        });
        builder.push(Op::update(counter, UpdateOp::AddInteger { value: 1 }, 2, 1));
    });
    assert!(
        machine
            .run()
            .unwrap_err()
            .message
            .contains("`+` cannot combine")
    );
}

#[test]
fn integer_compare_and_boolean_guard_cover_mismatched_types() {
    let mut machine = run(|builder| {
        let flag = builder.slot("flag");
        builder.push(Op::SetConst {
            slot: flag,
            value: Value::String("x".into()),
            line: 1,
        });
        let condition = builder.expr(&Expr::Variable("flag".into()), 2);
        builder.push(Op::JumpIfIntegerCompare {
            condition,
            slot: flag,
            comparison: BinaryOp::Equal,
            value: 1,
            target: 3,
        });
        builder.push(Op::Halt);
    });
    assert_eq!(machine.run().unwrap(), Yield::Finished);

    let mut machine = run(|builder| {
        let flag = builder.slot("flag");
        builder.push(Op::SetConst {
            slot: flag,
            value: Value::Integer(1),
            line: 1,
        });
        let condition = builder.expr(&Expr::Variable("flag".into()), 2);
        builder.push(Op::JumpIfFalse {
            condition,
            target: 3,
        });
        builder.push(Op::Halt);
    });
    assert!(
        machine
            .run()
            .unwrap_err()
            .message
            .contains("condition expects boolean")
    );
}

#[test]
fn restart_rejects_mismatched_frames_and_images_round_trip() {
    let mut builder = ProgramBuilder::new();
    let slot = builder.slot("value");
    builder.push(Op::SetConst {
        slot,
        value: Value::Integer(1),
        line: 1,
    });
    builder.push(Op::Halt);
    let program = builder.build();
    let mut machine = Machine::new(program.clone()).unwrap();
    let empty = InitialFrame::from_named_values(&velin_bytecode::SlotTable::new(), []).unwrap();
    assert_eq!(
        machine.restart(&empty, 0).unwrap_err(),
        "initial frame width does not match program slots"
    );

    let image = program.clone().into_execution_image();
    let mut from_image = Machine::new_execution_image(image).unwrap();
    assert_eq!(from_image.run().unwrap(), Yield::Finished);

    let validated = ValidatedProgram::new(program).unwrap();
    let from_validated = Machine::from_validated(&validated);
    assert_eq!(from_validated.variable("value"), None);

    assert_eq!(
        SetVariableError::UnknownVariable("x".into()).to_string(),
        "unknown variable `x`"
    );
    assert_eq!(SetVariableError::InvalidValue("bad").to_string(), "bad");
    assert_eq!(SetVariableError::StateBudget("full").to_string(), "full");

    let profile = ExecutionProfile::new(1);
    let other = ExecutionProfile::new(2);
    let mut merged = profile.clone();
    assert!(merged.merge(&other).is_err());
}

#[test]
fn prevalidated_machine_rejects_same_width_different_slot_layout() {
    let mut first_slots = velin_bytecode::SlotTable::new();
    first_slots.intern("hp");
    first_slots.intern("name");
    let first = Program {
        ops: Vec::new(),
        chunks: Vec::new(),
        expr_ops: Vec::new(),
        constants: Vec::new(),
        slots: first_slots,
    };
    let mut second_slots = velin_bytecode::SlotTable::new();
    second_slots.intern("name");
    second_slots.intern("hp");
    let second = Program {
        ops: Vec::new(),
        chunks: Vec::new(),
        expr_ops: Vec::new(),
        constants: Vec::new(),
        slots: second_slots,
    };
    let frame = InitialFrame::from_named_values(&first.slots, []).unwrap();
    let validated = ValidatedProgram::new(second).unwrap();
    assert!(Machine::from_validated_with_seed_and_frame(&validated, 0, &frame).is_none());
}

#[test]
fn batch_steps_non_host_ops_and_reports_runaway_loops() {
    let mut machine = run(|builder| {
        builder.push(Op::Jump(0));
    });
    assert!(
        machine
            .run_effect_batch(8)
            .unwrap_err()
            .message
            .contains("infinite loop")
    );

    let mut machine = run(|builder| {
        let boom = builder.expr(
            &Expr::Binary {
                left: Box::new(Expr::Value(Value::Integer(1))),
                op: BinaryOp::Divide,
                right: Box::new(Expr::Value(Value::Integer(0))),
            },
            1,
        );
        builder.push(Op::host(1, vec![boom], None, 1));
    });
    assert!(machine.run_effect_batch(8).is_err());
}

#[test]
fn integer_add_update_overflows_and_boolean_guards_take_both_edges() {
    let mut machine = run(|builder| {
        let counter = builder.slot("counter");
        builder.push(Op::SetConst {
            slot: counter,
            value: Value::Integer(i64::MAX),
            line: 1,
        });
        let rhs = builder.expr(&Expr::Value(Value::Integer(1)), 2);
        builder.push(Op::update(counter, UpdateOp::Add { rhs }, 2, 1));
    });
    assert_eq!(machine.run().unwrap_err().message, "integer overflow");

    for (flag, expected) in [(true, 2), (false, 1)] {
        let mut machine = run(|builder| {
            let enabled = builder.slot("enabled");
            let value = builder.slot("value");
            builder.push(Op::SetConst {
                slot: enabled,
                value: Value::Boolean(flag),
                line: 1,
            });
            builder.push(Op::SetConst {
                slot: value,
                value: Value::Integer(1),
                line: 1,
            });
            let condition = builder.expr(&Expr::Variable("enabled".into()), 2);
            let jump = builder.push(Op::JumpIfFalse {
                condition,
                target: u32::MAX,
            });
            builder.push(Op::SetConst {
                slot: value,
                value: Value::Integer(2),
                line: 3,
            });
            let end = builder.here();
            builder.patch(
                jump,
                Op::JumpIfFalse {
                    condition,
                    target: end,
                },
            );
        });
        assert_eq!(machine.run().unwrap(), Yield::Finished);
        assert_eq!(machine.variable("value"), Some(&Value::Integer(expected)));
    }

    for comparison in [
        BinaryOp::NotEqual,
        BinaryOp::Less,
        BinaryOp::LessEqual,
        BinaryOp::Greater,
        BinaryOp::GreaterEqual,
    ] {
        let mut machine = run(|builder| {
            let slot = builder.slot("slot");
            builder.push(Op::SetConst {
                slot,
                value: Value::Integer(2),
                line: 1,
            });
            let condition = builder.expr(&Expr::Variable("slot".into()), 2);
            let jump = builder.push(Op::JumpIfIntegerCompare {
                condition,
                slot,
                comparison,
                value: 2,
                target: u32::MAX,
            });
            let end = builder.here();
            builder.patch(
                jump,
                Op::JumpIfIntegerCompare {
                    condition,
                    slot,
                    comparison,
                    value: 2,
                    target: end,
                },
            );
        });
        assert_eq!(machine.run().unwrap(), Yield::Finished);
    }
}

#[test]
fn update_eval_errors_depth_budget_and_mismatched_frames() {
    let mut machine = run(|builder| {
        let items = builder.slot("items");
        builder.push(Op::SetConst {
            slot: items,
            value: Value::List(std::sync::Arc::new(Vec::new())),
            line: 1,
        });
        let boom = builder.expr(
            &Expr::Binary {
                left: Box::new(Expr::Value(Value::Integer(1))),
                op: BinaryOp::Divide,
                right: Box::new(Expr::Value(Value::Integer(0))),
            },
            2,
        );
        builder.push(Op::update(items, UpdateOp::Push { value: boom }, 2, 1));
    });
    assert!(machine.run().unwrap_err().message.contains("division"));

    let mut deep = Value::Integer(1);
    for _ in 0..16 {
        deep = Value::List(std::sync::Arc::new(vec![deep]));
    }
    let mut machine = run(|builder| {
        let items = builder.slot("items");
        builder.push(Op::SetConst {
            slot: items,
            value: Value::List(std::sync::Arc::new(Vec::new())),
            line: 1,
        });
        let value = builder.expr(&Expr::Variable("items".into()), 2);
        builder.push(Op::update(items, UpdateOp::Push { value }, 2, 1));
    });
    machine
        .try_set_variable("items", Value::List(std::sync::Arc::new(vec![deep])))
        .ok();
    let _ = machine.run();

    let huge = "x".repeat(velin_syntax::MAX_DATA_TEXT_BYTES / 2 + 2);
    let mut machine = run(|builder| {
        let name = builder.slot("name");
        builder.push(Op::SetConst {
            slot: name,
            value: Value::String(huge.clone().into()),
            line: 1,
        });
        let rhs = builder.expr(&Expr::Value(Value::String(huge.into())), 2);
        builder.push(Op::update(name, UpdateOp::Add { rhs }, 2, 1));
    });
    assert!(machine.run().is_err());

    let mut machine = run(|builder| {
        let counter = builder.slot("counter");
        builder.push(Op::update(counter, UpdateOp::AddInteger { value: 1 }, 1, 1));
    });
    assert!(machine.run().is_err());

    let mut machine = run(|builder| {
        let flag = builder.slot("flag");
        builder.push(Op::SetConst {
            slot: flag,
            value: Value::String("x".into()),
            line: 1,
        });
        let condition = builder.expr(&Expr::Variable("flag".into()), 2);
        let jump = builder.push(Op::JumpIfIntegerCompare {
            condition,
            slot: flag,
            comparison: BinaryOp::NotEqual,
            value: 1,
            target: u32::MAX,
        });
        let end = builder.here();
        builder.patch(
            jump,
            Op::JumpIfIntegerCompare {
                condition,
                slot: flag,
                comparison: BinaryOp::NotEqual,
                value: 1,
                target: end,
            },
        );
    });
    assert_eq!(machine.run().unwrap(), Yield::Finished);

    let mut builder = ProgramBuilder::new();
    builder.slot("value");
    let program = builder.build();
    let validated = ValidatedProgram::new(program).unwrap();
    let empty = InitialFrame::from_named_values(&velin_bytecode::SlotTable::new(), []).unwrap();
    assert!(Machine::from_validated_with_seed_and_frame(&validated, 0, &empty).is_none());
}
