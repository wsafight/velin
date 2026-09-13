use super::*;
use velin_bytecode::UpdateOp;
use velin_compile::ProgramBuilder;
use velin_syntax::{BinaryOp, Expr};

fn run(source_ops: impl FnOnce(&mut ProgramBuilder)) -> Machine {
    let mut builder = ProgramBuilder::new();
    source_ops(&mut builder);
    Machine::new(builder.build()).expect("valid program")
}

#[test]
fn integer_update_recomputes_footprint_when_the_cached_size_is_stale() {
    let mut machine = run(|builder| {
        let counter = builder.slot("counter");
        builder.push(Op::SetConst {
            slot: counter,
            value: Value::Integer(1),
            line: 1,
        });
        builder.push(Op::update(counter, UpdateOp::AddInteger { value: 1 }, 2, 1));
    });
    assert!(machine.step().unwrap().is_none());
    let index = machine.program.slots.get("counter").expect("counter slot") as usize;
    machine.frame.footprints[index] = DataFootprint {
        values: 0,
        text_bytes: 0,
    };
    assert!(machine.step().unwrap().is_none());
    assert_eq!(machine.variable("counter"), Some(&Value::Integer(2)));
}

#[test]
#[allow(clippy::too_many_lines)]
fn put_remove_eval_errors_and_over_deep_children_are_rejected() {
    fn div_zero(builder: &mut ProgramBuilder, _zero: u32) -> u32 {
        builder.expr(
            &Expr::Binary {
                left: Box::new(Expr::Value(Value::Integer(1))),
                op: BinaryOp::Divide,
                right: Box::new(Expr::Variable("zero".into())),
            },
            2,
        )
    }

    let mut machine = run(|builder| {
        let items = builder.slot("items");
        let zero = builder.slot("zero");
        builder.push(Op::SetConst {
            slot: items,
            value: Value::List(std::sync::Arc::new(vec![Value::Integer(1)])),
            line: 1,
        });
        builder.push(Op::SetConst {
            slot: zero,
            value: Value::Integer(0),
            line: 1,
        });
        let key = div_zero(builder, zero);
        let value = builder.expr(&Expr::Value(Value::Integer(2)), 2);
        builder.push(Op::update(items, UpdateOp::Put { key, value }, 2, 1));
    });
    assert!(machine.run().is_err());

    let mut machine = run(|builder| {
        let items = builder.slot("items");
        let zero = builder.slot("zero");
        builder.push(Op::SetConst {
            slot: items,
            value: Value::List(std::sync::Arc::new(vec![Value::Integer(1)])),
            line: 1,
        });
        builder.push(Op::SetConst {
            slot: zero,
            value: Value::Integer(0),
            line: 1,
        });
        let key = div_zero(builder, zero);
        builder.push(Op::update(items, UpdateOp::Remove { key }, 2, 1));
    });
    assert!(machine.run().is_err());

    let mut deep = Value::Integer(1);
    for _ in 0..16 {
        deep = Value::List(std::sync::Arc::new(vec![deep]));
    }
    let mut machine = run(|builder| {
        let items = builder.slot("items");
        builder.slot("child");
        builder.push(Op::SetConst {
            slot: items,
            value: Value::List(std::sync::Arc::new(Vec::new())),
            line: 1,
        });
        let value = builder.expr(&Expr::Variable("child".into()), 2);
        builder.push(Op::update(items, UpdateOp::Push { value }, 2, 1));
    });
    let child = machine.program.slots.get("child").expect("child") as usize;
    machine.frame.values[child] = Some(deep);
    machine.frame.footprints[child] = DataFootprint {
        values: 17,
        text_bytes: 0,
    };
    machine.frame.depths[child] = 16;
    assert!(machine.run().is_err());

    let mut machine = run(|builder| {
        let items = builder.slot("items");
        let zero = builder.slot("zero");
        builder.push(Op::SetConst {
            slot: items,
            value: Value::List(std::sync::Arc::new(vec![Value::Integer(1)])),
            line: 1,
        });
        builder.push(Op::SetConst {
            slot: zero,
            value: Value::Integer(0),
            line: 1,
        });
        let key = builder.expr(&Expr::Value(Value::Integer(0)), 2);
        let value = div_zero(builder, zero);
        builder.push(Op::update(items, UpdateOp::Put { key, value }, 2, 1));
    });
    assert!(machine.run().is_err());

    let mut machine = run(|builder| {
        let name = builder.slot("name");
        let zero = builder.slot("zero");
        builder.push(Op::SetConst {
            slot: name,
            value: Value::String("a".into()),
            line: 1,
        });
        builder.push(Op::SetConst {
            slot: zero,
            value: Value::Integer(0),
            line: 1,
        });
        let rhs = div_zero(builder, zero);
        builder.push(Op::update(name, UpdateOp::Add { rhs }, 2, 1));
    });
    assert!(machine.run().is_err());

    let mut machine = run(|builder| {
        builder.push(Op::Halt);
    });
    machine.pc = machine.program.ops.len();
    assert!(machine.run_effect_batch(4).unwrap().is_empty());

    let mut machine = run(|builder| {
        let cond = builder.slot("cond");
        builder.push(Op::SetConst {
            slot: cond,
            value: Value::Boolean(true),
            line: 1,
        });
        let condition = builder.expr(
            &Expr::Binary {
                left: Box::new(Expr::Variable("cond".into())),
                op: BinaryOp::And,
                right: Box::new(Expr::Value(Value::Boolean(true))),
            },
            2,
        );
        let jump = builder.push(Op::JumpIfFalse {
            condition,
            target: u32::MAX,
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
}

#[test]
fn batch_zero_limit_and_copy_slot_are_executed() {
    let mut machine = run(|builder| {
        builder.push(Op::host(1, Vec::new(), None, 1));
    });
    assert!(machine.run_effect_batch(0).is_err());

    let mut machine = run(|builder| {
        let source = builder.slot("source");
        let copy = builder.slot("copy");
        builder.push(Op::SetConst {
            slot: source,
            value: Value::Integer(4),
            line: 1,
        });
        builder.push(Op::CopySlot {
            slot: copy,
            source,
            line: 2,
            column: 1,
        });
        builder.push(Op::Halt);
    });
    assert_eq!(machine.run().unwrap(), Yield::Finished);
    assert_eq!(machine.variable("copy"), Some(&Value::Integer(4)));

    let mut machine = run(|builder| {
        let source = builder.slot("source");
        let copy = builder.slot("copy");
        builder.push(Op::SetConst {
            slot: source,
            value: Value::List(std::sync::Arc::new(vec![Value::Integer(1)])),
            line: 1,
        });
        builder.push(Op::CopySlot {
            slot: copy,
            source,
            line: 2,
            column: 1,
        });
    });
    assert_eq!(machine.run().unwrap(), Yield::Finished);
    assert!(machine.variable("copy").is_some());
}
