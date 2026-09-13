use super::*;
use crate::SlotTable;
use velin_syntax::Value;

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

#[test]
fn validates_stack_paths_and_rejects_back_edges() {
    let valid = ExprChunk {
        ops: vec![ExprOp::Const(0)],
        constants: vec![Value::Integer(1)],
        line: 1,
    };
    assert!(program(valid).validate().is_ok());

    let looping = ExprChunk {
        ops: vec![ExprOp::Const(0), ExprOp::JumpIfTrue(0)],
        constants: vec![Value::Boolean(true)],
        line: 1,
    };
    assert!(
        program(looping)
            .validate()
            .unwrap_err()
            .message
            .contains("non-forward")
    );

    let underflow = ExprChunk {
        ops: vec![ExprOp::Binary(BinaryOp::Add)],
        constants: Vec::new(),
        line: 1,
    };
    assert!(
        program(underflow)
            .validate()
            .unwrap_err()
            .message
            .contains("needs 2")
    );

    let inconsistent = ExprChunk {
        ops: vec![ExprOp::Const(0), ExprOp::JumpIfTrue(3), ExprOp::Const(0)],
        constants: vec![Value::Boolean(true)],
        line: 1,
    };
    assert!(
        program(inconsistent)
            .validate()
            .unwrap_err()
            .message
            .contains("inconsistent stack heights")
    );
}

#[test]
fn rejects_invalid_program_indices() {
    let chunk = ExprChunk {
        ops: vec![ExprOp::Const(0)],
        constants: vec![Value::Integer(1)],
        line: 1,
    };
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

    let mut invalid_range = program(ExprChunk {
        ops: vec![ExprOp::Const(0)],
        constants: vec![Value::Integer(1)],
        line: 1,
    });
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
        ops: vec![ExprOp::Load { slot: 1, column: 1 }],
        constants: Vec::new(),
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
        ops: vec![ExprOp::Const(0)],
        constants: vec![Value::String("x".repeat(MAX_PROGRAM_TEXT_BYTES + 1).into())],
        line: 1,
    };
    assert!(oversized.validate(0).is_err());
}

#[test]
fn validated_program_requires_a_valid_program_and_keeps_it_shared() {
    let valid = Arc::new(program(ExprChunk {
        ops: vec![ExprOp::Const(0)],
        constants: vec![Value::Integer(1)],
        line: 1,
    }));
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
