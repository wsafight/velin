use super::*;
use velin_syntax::{BinaryOp, Builtin, Expr, Value};

#[test]
fn builder_shares_slots_across_expressions() {
    let mut builder = ProgramBuilder::new();
    let hp = builder.slot("hp");
    let e1 = builder.expr(&Expr::Variable("hp".into()), 1);
    let e2 = builder.expr(
        &Expr::Binary {
            left: Box::new(Expr::Variable("hp".into())),
            op: BinaryOp::Subtract,
            right: Box::new(Expr::Value(Value::Integer(1))),
        },
        2,
    );
    assert_ne!(e1, e2);
    let program = builder.build();
    assert_eq!(program.slots.get("hp"), Some(hp));
    assert!(matches!(program.ops.last(), Some(Op::Halt)));
    assert_eq!(program.chunks.len(), 2);
    assert_eq!(program.expr_ops.len(), 4);
    assert_eq!(program.chunk(e1).unwrap().ops.len(), 1);
    assert_eq!(program.chunk(e2).unwrap().ops.len(), 3);
}

#[test]
fn builder_specializes_constant_copies_and_integer_guards() {
    let mut builder = ProgramBuilder::new();
    let source = builder.slot("source");
    let target = builder.slot("target");

    let constant = builder.set_op(target, &Expr::Value(Value::Integer(7)), 2);
    assert!(matches!(
        constant,
        Op::SetConst {
            slot,
            value: Value::Integer(7),
            line: 2
        } if slot == target
    ));
    let copy = builder.set_op(target, &Expr::Variable("source".into()), 3);
    assert!(matches!(
        copy,
        Op::CopySlot {
            slot,
            source: copy_source,
            line: 3,
            ..
        } if slot == target && copy_source == source
    ));
    assert!(builder.chunks.is_empty());

    let comparison = Expr::Binary {
        left: Box::new(Expr::Variable("source".into())),
        op: BinaryOp::Less,
        right: Box::new(Expr::Value(Value::Integer(10))),
    };
    let guard = builder.jump_if_false_op(&comparison, 4, 9);
    assert!(matches!(
        guard,
        Op::JumpIfIntegerCompare {
            slot,
            comparison: BinaryOp::Less,
            value: 10,
            target: 9,
            ..
        } if slot == source
    ));
    assert_eq!(builder.chunks.len(), 1);
}

#[test]
fn builder_propagates_constants_only_within_a_linear_region() {
    let mut builder = ProgramBuilder::new();
    let source = builder.slot("source");
    let target = builder.slot("target");
    let source_set = builder.set_op(source, &Expr::Value(Value::Integer(4)), 1);
    builder.push(source_set);

    let propagated = builder.set_op(
        target,
        &Expr::Binary {
            left: Box::new(Expr::Variable("source".into())),
            op: BinaryOp::Add,
            right: Box::new(Expr::Value(Value::Integer(3))),
        },
        2,
    );
    assert!(matches!(
        propagated,
        Op::SetConst {
            value: Value::Integer(7),
            ..
        }
    ));
    builder.push(propagated);

    builder.push(Op::Jump(0));
    let after_jump = builder.set_op(
        target,
        &Expr::Binary {
            left: Box::new(Expr::Variable("source".into())),
            op: BinaryOp::Add,
            right: Box::new(Expr::Value(Value::Integer(3))),
        },
        3,
    );
    assert!(matches!(after_jump, Op::Set { .. }));
}

#[test]
fn builder_does_not_propagate_random_expressions() {
    let mut builder = ProgramBuilder::new();
    let target = builder.slot("target");
    let initial = builder.set_op(target, &Expr::Value(Value::Integer(1)), 1);
    builder.push(initial);
    let random = Expr::Invoke {
        function: Builtin::Random,
        arguments: vec![
            Expr::Value(Value::Integer(1)),
            Expr::Value(Value::Integer(2)),
        ],
    };
    let operation = builder.set_op(target, &random, 2);
    assert!(matches!(operation, Op::Set { .. }));
}

#[test]
fn builder_folds_constant_guards_and_threads_jump_chains() {
    let mut builder = ProgramBuilder::new();
    let condition = builder.expr(&Expr::Value(Value::Boolean(false)), 1);
    builder.push(Op::JumpIfFalse {
        condition,
        target: 1,
    });
    builder.push(Op::Jump(3));
    builder.push(Op::Halt);
    builder.push(Op::Halt);
    let program = builder.build();
    assert_eq!(program.ops[0], Op::Jump(3));

    let mut cyclic = ProgramBuilder::new();
    cyclic.push(Op::Jump(1));
    cyclic.push(Op::Jump(0));
    let program = cyclic.build();
    assert_eq!(program.ops, vec![Op::Jump(1), Op::Jump(0)]);
}

#[test]
fn validation_precomputes_execution_metadata() {
    let mut builder = ProgramBuilder::new();
    let target = builder.slot("target");
    let expression = Expr::Binary {
        left: Box::new(Expr::Variable("target".into())),
        op: BinaryOp::Add,
        right: Box::new(Expr::Value(Value::Integer(1))),
    };
    let chunk = builder.expr(&expression, 6);
    builder.push(Op::Set {
        slot: target,
        value: chunk,
    });
    let constant = builder.set_op(target, &Expr::Value(Value::String("abc".into())), 7);
    builder.push(constant);
    let validated = crate::ValidatedProgram::new(builder.build()).unwrap();
    let metadata = validated.shared_execution_metadata();

    assert_eq!(metadata.chunk(chunk).unwrap().registers, 2);
    assert!(!metadata.chunk(chunk).unwrap().mutates_frame);
    assert_eq!(metadata.op(0).unwrap().line, 6);
    assert_eq!(metadata.op(1).unwrap().line, 7);
    assert_eq!(
        metadata
            .op_constant_metrics(1)
            .unwrap()
            .footprint
            .text_bytes,
        3
    );
    assert_eq!(metadata.program_constant_metrics().len(), 1);
}

#[test]
fn nameless_programs_round_trip_with_slot_width_and_rng_state() {
    let mut builder = ProgramBuilder::new();
    let result = builder.slot("result");
    let random = Expr::Invoke {
        function: Builtin::Random,
        arguments: vec![
            Expr::Value(Value::Integer(1)),
            Expr::Value(Value::Integer(6)),
        ],
    };
    let chunk = builder.expr(&random, 1);
    builder.push(Op::Set {
        slot: result,
        value: chunk,
    });
    let program = builder.build();
    let image = program.clone().into_execution_image();
    let json = serde_json::to_string(image.program()).unwrap();
    let restored: Program = serde_json::from_str(&json).unwrap();

    assert_eq!(restored.slots.len(), program.slots.len());
    assert_eq!(restored.slots.rng_state(), program.slots.rng_state());
    assert_eq!(restored.ops, program.ops);
}

#[test]
fn compiler_emits_register_bytecode_for_long_scalar_expressions() {
    let mut builder = ProgramBuilder::new();
    let _left = builder.slot("left");
    let _right = builder.slot("right");
    let target = builder.slot("target");
    let mut expression = Expr::Variable("left".into());
    for _ in 0..8 {
        expression = Expr::Binary {
            left: Box::new(expression),
            op: BinaryOp::Add,
            right: Box::new(Expr::Variable("right".into())),
        };
    }
    let chunk = builder.expr(&expression, 3);
    builder.push(Op::Set {
        slot: target,
        value: chunk,
    });
    let program = builder.build();
    let bytecode = program.chunk(chunk).unwrap();
    assert_eq!(bytecode.result, 0);
    assert!(bytecode.registers >= 9);
    assert!(
        bytecode
            .ops
            .iter()
            .any(|op| matches!(op, ExprOp::Binary { .. }))
    );
}

#[test]
fn register_bytecode_names_every_scalar_operand() {
    let mut builder = ProgramBuilder::new();
    let _source = builder.slot("source");
    let target = builder.slot("target");
    let mut expression = Expr::Variable("source".into());
    for _ in 0..6 {
        expression = Expr::Binary {
            left: Box::new(expression),
            op: BinaryOp::Subtract,
            right: Box::new(Expr::Value(Value::Integer(1))),
        };
    }
    let chunk = builder.expr(&expression, 3);
    builder.push(Op::Set {
        slot: target,
        value: chunk,
    });
    let program = builder.build();
    let bytecode = program.chunk(chunk).unwrap();
    assert!(bytecode.ops.iter().any(|operation| matches!(
        operation,
        ExprOp::Binary {
            dst: 0,
            left: 0,
            op: BinaryOp::Subtract,
            ..
        }
    )));
}

#[test]
fn builtins_use_contiguous_register_arguments() {
    let mut builder = ProgramBuilder::new();
    let items = builder.slot("items");
    let target = builder.slot("target");
    let expression = Expr::Invoke {
        function: Builtin::Contains,
        arguments: vec![
            Expr::Variable("items".into()),
            Expr::Value(Value::Integer(2)),
        ],
    };
    let chunk = builder.expr(&expression, 4);
    builder.push(Op::Set {
        slot: target,
        value: chunk,
    });
    let program = builder.build();
    let bytecode = program.chunk(chunk).unwrap();
    assert!(matches!(
        bytecode.ops,
        [
            ExprOp::Load { dst: 1, slot, .. },
            ExprOp::Const {
                dst: 2,
                constant: 0
            },
            ExprOp::Call {
                dst: 0,
                function: Builtin::Contains,
                args
            }
        ] if *slot == items && args == &(1..3)
    ));
}

#[test]
#[cfg(target_pointer_width = "64")]
fn bytecode_layout_stays_compact() {
    assert_eq!(std::mem::size_of::<crate::ExprOp>(), 12);
    assert_eq!(std::mem::size_of::<Op>(), 32);
    assert_eq!(std::mem::size_of::<ProgramChunk>(), 24);
    assert_eq!(std::mem::size_of::<ExprChunk>(), 56);
}

#[test]
fn programs_round_trip_through_json() {
    let mut builder = ProgramBuilder::new();
    let slot = builder.slot("x");
    let value = builder.expr(&Expr::Value(Value::Integer(7)), 1);
    builder.push(Op::Set { slot, value });
    let program = builder.build();
    let json = serde_json::to_string(&program).unwrap();
    let restored: Program = serde_json::from_str(&json).unwrap();
    assert_eq!(program, restored);
}

#[test]
fn boxed_host_payload_keeps_the_existing_json_shape() {
    let mut builder = ProgramBuilder::new();
    builder.push(Op::host(7, Vec::new(), None, 3));
    let program = builder.build();
    let json = serde_json::to_value(&program).unwrap();
    assert_eq!(
        json["ops"][0]["Host"],
        serde_json::json!({
            "host_id": 7,
            "args": [],
            "bind": null,
            "line": 3
        })
    );
    let restored: Program = serde_json::from_value(json).unwrap();
    assert_eq!(program, restored);
}

#[test]
fn random_programs_round_trip_with_their_state_slot() {
    let mut builder = ProgramBuilder::new();
    let roll = builder.slot("roll");
    let expression = velin_parse::parse_expression("random(1, 6)", "test", 1, 1).unwrap();
    let value = builder.expr(&expression, 1);
    builder.push(Op::Set { slot: roll, value });
    let program = builder.build();
    let rng_slot = program.slots.rng_state().expect("RNG slot serialized");
    let json = serde_json::to_string(&program).unwrap();
    let restored: Program = serde_json::from_str(&json).unwrap();
    assert_eq!(program, restored);
    assert_eq!(restored.slots.rng_state(), Some(rng_slot));
    assert!(matches!(
        restored.chunk(0).unwrap().ops.last(),
        Some(crate::ExprOp::Random { state_slot, .. }) if *state_slot == rng_slot
    ));
}

#[test]
fn deserialization_rejects_malformed_programs_and_duplicate_slots() {
    let missing_chunk = r#"{
            "ops":[{"Set":{"slot":0,"value":9}}],
            "chunks":[],
            "slots":["x"]
        }"#;
    let error = serde_json::from_str::<Program>(missing_chunk).unwrap_err();
    assert!(error.to_string().contains("missing expression chunk"));

    let duplicate_slots = r#"{
            "ops":["Halt"],
            "chunks":[],
            "slots":["x","x"]
        }"#;
    let error = serde_json::from_str::<Program>(duplicate_slots).unwrap_err();
    assert!(error.to_string().contains("duplicate slot name"));
}
