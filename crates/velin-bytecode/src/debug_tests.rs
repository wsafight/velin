use super::*;
use crate::{ExprChunk, ExprOp, Op, Program, SlotTable};
use velin_syntax::Value;

fn load_chunk(slot: u32, column: u32) -> ExprChunk {
    ExprChunk {
        ops: vec![ExprOp::Load {
            dst: 0,
            slot,
            column,
        }],
        constants: Vec::new(),
        registers: 1,
        result: 0,
        line: 4,
    }
}

#[test]
fn covers_every_operation_location_shape_and_table_extents() {
    let mut slots = SlotTable::new();
    let target = slots.intern("target");
    let source = slots.intern("source");
    let program = Program::from_chunks(
        vec![
            Op::Set {
                slot: target,
                value: 0,
            },
            Op::SetConst {
                slot: target,
                value: Value::Integer(1),
                line: 7,
            },
            Op::JumpIfFalse {
                condition: 0,
                target: 5,
            },
            Op::JumpIfIntegerCompare {
                condition: 0,
                slot: source,
                comparison: velin_syntax::BinaryOp::Equal,
                value: 1,
                target: 6,
            },
            Op::CopySlot {
                slot: target,
                source,
                line: 4,
                column: 12,
            },
            Op::Update {
                slot: target,
                operation: crate::UpdateOp::AddInteger { value: 1 },
                line: 5,
                column: 3,
            },
            Op::host(9, Vec::new(), None, 6),
            Op::Jump(8),
            Op::Halt,
        ],
        vec![load_chunk(source, 9)],
        slots,
    );
    let debug = program.debug_table();
    assert!(!debug.is_empty());
    assert!(debug.len() > 9);
    assert_eq!(debug.chunk(0).unwrap().line, 4);
    assert!(debug.chunk(99).is_none());
    assert_eq!(debug.op(0).unwrap().line, 4);
    assert_eq!(debug.op(1).unwrap().line, 7);
    assert_eq!(debug.op(2).unwrap().line, 4);
    assert_eq!(debug.op(3).unwrap().line, 4);
    assert_eq!(debug.op(4).unwrap().column, 12);
    assert_eq!(debug.op(5).unwrap().line, 5);
    assert_eq!(debug.op(6).unwrap().line, 6);
    assert_eq!(debug.op(7).unwrap().line, 0);
    assert_eq!(debug.op(8).unwrap().line, 0);
    assert!(debug.op(99).is_none());
    assert_eq!(debug.expression_column(0), Some(9));
}

#[test]
fn missing_chunk_references_fall_back_to_zero_locations() {
    let program =
        Program::from_chunks(vec![Op::Set { slot: 0, value: 3 }, Op::Halt], Vec::new(), {
            let mut slots = SlotTable::new();
            slots.intern("x");
            slots
        });
    let debug = program.debug_table();
    assert_eq!(debug.op(0).unwrap(), DebugLocation { line: 0, column: 0 });
}

#[test]
fn expression_columns_are_zero_for_non_load_operations() {
    let mut slots = SlotTable::new();
    slots.intern("x");
    let program = Program::from_chunks(
        vec![Op::Halt],
        vec![ExprChunk {
            ops: vec![
                ExprOp::Const {
                    dst: 0,
                    constant: 0,
                },
                ExprOp::Load {
                    dst: 0,
                    slot: 0,
                    column: 0,
                },
            ],
            constants: vec![Value::Integer(1)],
            registers: 1,
            result: 0,
            line: 1,
        }],
        slots,
    );
    let debug = program.debug_table();
    assert_eq!(debug.expression_column(0), None);
}

#[test]
fn empty_program_debug_table_is_empty() {
    let program = Program::from_chunks(Vec::new(), Vec::new(), SlotTable::new());
    let debug = program.debug_table();
    assert!(debug.is_empty());
    assert_eq!(debug.len(), 0);
}
