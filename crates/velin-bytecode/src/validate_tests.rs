use super::*;
use crate::SlotTable;
use velin_syntax::{UnaryOp, Value};

fn program(chunk: ExprChunk) -> Program {
    Program::from_chunks(
        vec![Op::Set { slot: 0, value: 0 }, Op::Halt],
        vec![chunk],
        {
            let mut slots = SlotTable::new();
            slots.intern("x");
            slots
        },
    )
}

fn constant_chunk(value: Value) -> ExprChunk {
    ExprChunk {
        ops: vec![ExprOp::Const {
            dst: 0,
            constant: 0,
        }],
        constants: vec![value],
        registers: 1,
        result: 0,
        line: 1,
    }
}

#[test]
fn validates_register_flow_and_rejects_back_edges() {
    assert!(
        program(constant_chunk(Value::Integer(1)))
            .validate()
            .is_ok()
    );

    let looping = ExprChunk {
        ops: vec![
            ExprOp::Const {
                dst: 0,
                constant: 0,
            },
            ExprOp::JumpIfTrue {
                condition: 0,
                target: 0,
            },
        ],
        constants: vec![Value::Boolean(true)],
        registers: 1,
        result: 0,
        line: 1,
    };
    assert!(
        program(looping)
            .validate()
            .unwrap_err()
            .message
            .contains("non-forward")
    );

    let undefined_source = ExprChunk {
        ops: vec![ExprOp::Binary {
            dst: 0,
            left: 0,
            op: BinaryOp::Add,
            right: 1,
        }],
        constants: Vec::new(),
        registers: 2,
        result: 0,
        line: 1,
    };
    assert!(
        program(undefined_source)
            .validate()
            .unwrap_err()
            .message
            .contains("reads undefined register")
    );

    let missing_on_one_branch = ExprChunk {
        ops: vec![
            ExprOp::Const {
                dst: 0,
                constant: 0,
            },
            ExprOp::JumpIfTrue {
                condition: 0,
                target: 3,
            },
            ExprOp::Const {
                dst: 1,
                constant: 0,
            },
        ],
        constants: vec![Value::Boolean(true)],
        registers: 2,
        result: 1,
        line: 1,
    };
    assert!(
        program(missing_on_one_branch)
            .validate()
            .unwrap_err()
            .message
            .contains("leaves result register 1 undefined")
    );
}

#[test]
fn rejects_undefined_sources_for_every_register_instruction_shape() {
    let cases = [
        ExprOp::Unary {
            dst: 0,
            op: UnaryOp::Negate,
            source: 1,
        },
        ExprOp::Call {
            dst: 0,
            function: Builtin::Len,
            args: 1..2,
        },
        ExprOp::Random {
            dst: 0,
            args: 1..3,
            state_slot: 0,
        },
        ExprOp::Chance {
            dst: 0,
            args: 1..2,
            state_slot: 0,
        },
        ExprOp::Concat {
            dst: 0,
            values: 1..2,
        },
        ExprOp::JumpIfFalse {
            condition: 0,
            target: 1,
        },
        ExprOp::JumpIfTrue {
            condition: 0,
            target: 1,
        },
    ];

    for op in cases {
        let registers = match op {
            ExprOp::Random { .. } => 3,
            _ => 2,
        };
        let chunk = ExprChunk {
            ops: vec![op],
            constants: Vec::new(),
            registers,
            result: 0,
            line: 1,
        };
        assert!(
            program(chunk)
                .validate()
                .unwrap_err()
                .message
                .contains("reads undefined register")
        );
    }
}

#[test]
fn rejects_invalid_program_indices() {
    let chunk = constant_chunk(Value::Integer(1));
    let mut invalid = program(chunk);
    invalid.ops[0] = Op::Set { slot: 9, value: 0 };
    assert!(
        invalid
            .validate()
            .unwrap_err()
            .message
            .contains("missing slot")
    );
    invalid.ops[0] = Op::Jump(99);
    assert!(
        invalid
            .validate()
            .unwrap_err()
            .message
            .contains("past program end")
    );

    let mut invalid_range = program(constant_chunk(Value::Integer(1)));
    invalid_range.chunks[0].ops.end += 1;
    assert!(
        invalid_range
            .validate()
            .unwrap_err()
            .message
            .contains("invalid arena range")
    );
}

#[test]
fn standalone_chunk_validation_checks_frame_width_and_constants() {
    let missing_slot = ExprChunk {
        ops: vec![ExprOp::Load {
            dst: 0,
            slot: 1,
            column: 1,
        }],
        constants: Vec::new(),
        registers: 1,
        result: 0,
        line: 1,
    };
    assert!(
        missing_slot
            .validate(1)
            .unwrap_err()
            .message
            .contains("missing slot")
    );

    let oversized = ExprChunk {
        ops: vec![ExprOp::Const {
            dst: 0,
            constant: 0,
        }],
        constants: vec![Value::String("x".repeat(MAX_PROGRAM_TEXT_BYTES + 1).into())],
        registers: 1,
        result: 0,
        line: 1,
    };
    assert!(oversized.validate(0).is_err());
}

#[test]
fn random_ops_require_the_reserved_rng_slot_and_dedicated_encoding() {
    let random_chunk = ExprChunk {
        ops: vec![
            ExprOp::Const {
                dst: 1,
                constant: 0,
            },
            ExprOp::Const {
                dst: 2,
                constant: 1,
            },
            ExprOp::Random {
                dst: 0,
                args: 1..3,
                state_slot: 0,
            },
        ],
        constants: vec![Value::Integer(1), Value::Integer(6)],
        registers: 3,
        result: 0,
        line: 1,
    };
    let mut slots = SlotTable::new();
    slots.intern("user");
    slots.intern_rng_state();
    let program = Program::from_chunks(vec![Op::Halt], vec![random_chunk], slots);
    let error = program.validate().unwrap_err();
    assert!(error.message.contains("does not match reserved RNG slot"));

    let call_chunk = ExprChunk {
        ops: vec![
            ExprOp::Const {
                dst: 1,
                constant: 0,
            },
            ExprOp::Const {
                dst: 2,
                constant: 1,
            },
            ExprOp::Call {
                dst: 0,
                function: Builtin::Random,
                args: 1..3,
            },
        ],
        constants: vec![Value::Integer(1), Value::Integer(6)],
        registers: 3,
        result: 0,
        line: 1,
    };
    let mut slots = SlotTable::new();
    slots.intern_rng_state();
    let program = Program::from_chunks(vec![Op::Halt], vec![call_chunk], slots);
    let error = program.validate().unwrap_err();
    assert!(error.message.contains("dedicated random opcode"));
}

#[test]
fn rejects_invalid_register_ranges_before_execution() {
    let invalid_range = ExprChunk {
        ops: vec![ExprOp::Concat {
            dst: 0,
            values: 1..3,
        }],
        constants: Vec::new(),
        registers: 2,
        result: 0,
        line: 1,
    };
    assert!(
        invalid_range
            .validate(0)
            .unwrap_err()
            .message
            .contains("invalid register range")
    );
}

#[test]
fn validated_program_requires_a_valid_program_and_keeps_it_shared() {
    let valid = Arc::new(program(constant_chunk(Value::Integer(1))));
    let validated = ValidatedProgram::new(valid.clone()).unwrap();
    assert!(Arc::ptr_eq(&valid, &validated.shared()));

    let invalid = Program {
        ops: vec![Op::Jump(2)],
        chunks: Vec::new(),
        expr_ops: Vec::new(),
        constants: Vec::new(),
        slots: SlotTable::new(),
    };
    assert!(ValidatedProgram::new(invalid).is_err());
}

#[test]
fn execution_image_can_move_columns_to_a_debug_sidecar() {
    let mut slots = SlotTable::new();
    let source = slots.intern("source");
    let target = slots.intern("target");
    let program = Program::from_chunks(
        vec![
            Op::Set {
                slot: target,
                value: 0,
            },
            Op::CopySlot {
                slot: target,
                source,
                line: 4,
                column: 12,
            },
            Op::Halt,
        ],
        vec![ExprChunk {
            ops: vec![ExprOp::Load {
                dst: 0,
                slot: source,
                column: 9,
            }],
            constants: Vec::new(),
            registers: 1,
            result: 0,
            line: 3,
        }],
        slots,
    );
    let (image, debug) = program.without_debug_columns();
    assert_eq!(debug.chunk(0).unwrap().line, 3);
    assert_eq!(debug.op(0).unwrap().line, 3);
    assert_eq!(debug.op(1).unwrap().column, 12);
    assert_eq!(debug.expression_column(0), Some(9));
    assert!(matches!(
        image.chunk(0).unwrap().ops[0],
        ExprOp::Load { column: 0, .. }
    ));
    assert!(matches!(image.ops[1], Op::CopySlot { column: 0, .. }));
    assert!(image.validate().is_ok());
}

#[test]
fn execution_image_drops_slot_names_but_keeps_runtime_width() {
    let mut slots = SlotTable::new();
    slots.intern("value");
    let program = Program::from_chunks(vec![Op::Halt], Vec::new(), slots);
    let width = program.slots.len();
    let image = program.into_execution_image();
    assert_eq!(image.program().slots.len(), width);
    assert!(!image.program().slots.has_names());
    assert!(image.program().validate().is_ok());
    assert_eq!(image.debug().op(0).unwrap().line, 0);
}

#[test]
fn validates_every_program_operation_shape() {
    let mut slots = SlotTable::new();
    let target = slots.intern("target");
    let source = slots.intern("source");
    let chunk = constant_chunk(Value::Integer(1));
    let program = Program::from_chunks(
        vec![
            Op::Set {
                slot: target,
                value: 0,
            },
            Op::SetConst {
                slot: target,
                value: Value::Integer(1),
                line: 3,
            },
            Op::CopySlot {
                slot: target,
                source,
                line: 4,
                column: 2,
            },
            Op::Update {
                slot: target,
                operation: UpdateOp::Add { rhs: 0 },
                line: 5,
                column: 1,
            },
            Op::Update {
                slot: target,
                operation: UpdateOp::AddInteger { value: 1 },
                line: 6,
                column: 1,
            },
            Op::Update {
                slot: target,
                operation: UpdateOp::Push { value: 0 },
                line: 7,
                column: 1,
            },
            Op::Update {
                slot: target,
                operation: UpdateOp::Put { key: 0, value: 0 },
                line: 8,
                column: 1,
            },
            Op::Update {
                slot: target,
                operation: UpdateOp::Remove { key: 0 },
                line: 9,
                column: 1,
            },
            Op::JumpIfFalse {
                condition: 0,
                target: 10,
            },
            Op::JumpIfIntegerCompare {
                condition: 0,
                slot: source,
                comparison: BinaryOp::Equal,
                value: 1,
                target: 11,
            },
            Op::host(0, vec![0], Some(source), 11),
            Op::Halt,
        ],
        vec![chunk],
        slots,
    );
    assert!(program.validate().is_ok());
}

#[test]
fn rejects_invalid_integer_comparisons_and_host_slots() {
    let mut slots = SlotTable::new();
    let slot = slots.intern("x");
    let invalid_comparison = Program::from_chunks(
        vec![
            Op::JumpIfIntegerCompare {
                condition: 0,
                slot,
                comparison: BinaryOp::Add,
                value: 1,
                target: 2,
            },
            Op::Halt,
        ],
        vec![constant_chunk(Value::Integer(1))],
        slots.clone(),
    );
    assert!(
        invalid_comparison
            .validate()
            .unwrap_err()
            .message
            .contains("invalid integer comparison")
    );

    let invalid_host_slot = Program::from_chunks(
        vec![Op::host(0, Vec::new(), Some(9), 1), Op::Halt],
        Vec::new(),
        slots,
    );
    assert!(
        invalid_host_slot
            .validate()
            .unwrap_err()
            .message
            .contains("missing slot")
    );
}
