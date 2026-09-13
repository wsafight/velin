use super::*;
use crate::{ExprChunk, ExprOp, Op, SlotTable};
use velin_syntax::BinaryOp;

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
fn lowers_copy_set_host_and_integer_compare_ops() {
    let mut slots = SlotTable::new();
    let source = slots.intern("source");
    let target = slots.intern("target");
    let program = Program::from_chunks(
        vec![
            Op::SetConst {
                slot: source,
                value: Value::Integer(1),
                line: 1,
            },
            Op::CopySlot {
                slot: target,
                source,
                line: 2,
                column: 1,
            },
            Op::Set {
                slot: target,
                value: 0,
            },
            Op::host(1, Vec::new(), Some(target), 3),
            Op::JumpIfIntegerCompare {
                condition: 0,
                slot: source,
                comparison: BinaryOp::Equal,
                value: 1,
                target: 6,
            },
            Op::Update {
                slot: source,
                operation: UpdateOp::AddInteger { value: 1 },
                line: 4,
                column: 1,
            },
            Op::Halt,
        ],
        vec![constant_chunk(Value::Integer(1))],
        slots,
    );
    let ir = TypedIr::from_program(&program);
    assert!(ir.value_count() > 0);
    assert_eq!(ir.entry(), 0);
    assert!(!ir.blocks().is_empty());
    let _ = ir.constant(IrValue(0));
    let _ = ir.loops();
    let _ = ir.optimizations();
    assert!(ir.blocks().iter().any(|block| {
        block.operations.iter().any(|operation| {
            matches!(
                operation,
                IrOp::Slot { .. }
                    | IrOp::Evaluate { .. }
                    | IrOp::Unknown { .. }
                    | IrOp::Update { .. }
            )
        })
    }));
}

#[test]
fn constant_lookup_and_value_types_cover_compound_values() {
    let mut slots = SlotTable::new();
    let slot = slots.intern("value");
    let program = Program::from_chunks(
        vec![
            Op::SetConst {
                slot,
                value: Value::List(std::sync::Arc::new(vec![Value::Integer(1)])),
                line: 1,
            },
            Op::Halt,
        ],
        Vec::new(),
        slots,
    );
    let ir = TypedIr::from_program(&program);
    assert!(ir.constant(IrValue(ir.value_count())).is_none());
    assert!(
        ir.blocks()
            .iter()
            .any(|block| block.operations.iter().any(|operation| matches!(
                operation,
                IrOp::Constant {
                    value_type: IrType::Compound,
                    ..
                }
            )))
    );
}

#[test]
fn execution_images_and_nameless_programs_preserve_ops() {
    let mut slots = SlotTable::new();
    let slot = slots.intern("value");
    let program = Program::from_chunks(
        vec![
            Op::SetConst {
                slot,
                value: Value::Integer(1),
                line: 1,
            },
            Op::Halt,
        ],
        Vec::new(),
        slots,
    );
    let nameless = program.clone().without_slot_names();
    assert_eq!(nameless.ops.len(), program.ops.len());
    let image = program.clone().into_execution_image();
    assert_eq!(image.program().ops.len(), program.ops.len());
    let _ = image.debug();
    let restored = image.into_program();
    assert_eq!(restored.ops.len(), program.ops.len());
    let metadata = crate::ValidatedProgram::new(program)
        .unwrap()
        .shared_execution_metadata();
    assert!(metadata.chunk(99).is_none());
    assert!(metadata.chunk_constant_metrics(99).is_none());
    assert!(metadata.chunk_with_constant_metrics(99).is_none());
}
