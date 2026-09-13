use super::*;
use crate::{ExprChunk, ExprOp, Op, SlotTable};

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
fn builds_real_phi_values_at_control_flow_joins() {
    let mut slots = SlotTable::new();
    let value = slots.intern("value");
    let condition = slots.intern("condition");
    let program = Program::from_chunks(
        vec![
            Op::SetConst {
                slot: condition,
                value: Value::Boolean(true),
                line: 1,
            },
            Op::SetConst {
                slot: value,
                value: Value::Integer(1),
                line: 1,
            },
            Op::JumpIfFalse {
                condition: 0,
                target: 5,
            },
            Op::SetConst {
                slot: value,
                value: Value::Integer(2),
                line: 2,
            },
            Op::Jump(6),
            Op::SetConst {
                slot: value,
                value: Value::Integer(3),
                line: 3,
            },
            Op::Halt,
        ],
        vec![constant_chunk(Value::Boolean(true))],
        slots,
    );
    let ir = TypedIr::from_program(&program);
    assert!(ir.blocks().iter().any(|block| {
        block.operations.iter().any(|operation| {
            matches!(operation, IrOp::Phi { slot, inputs, .. } if *slot == value && inputs.len() == 2)
        })
    }));
    assert!(
        ir.optimizations()
            .iter()
            .any(|optimization| matches!(optimization, IrOptimization::BranchFolded { .. }))
    );
}

#[test]
fn folds_constants_and_discovers_loop_strength_reduction() {
    let mut slots = SlotTable::new();
    let counter = slots.intern("counter");
    let condition = slots.intern("condition");
    let invariant = slots.intern("invariant");
    let program = Program::from_chunks(
        vec![
            Op::SetConst {
                slot: counter,
                value: Value::Integer(0),
                line: 1,
            },
            Op::JumpIfFalse {
                condition: 0,
                target: 5,
            },
            Op::SetConst {
                slot: invariant,
                value: Value::Integer(7),
                line: 2,
            },
            Op::Update {
                slot: counter,
                operation: UpdateOp::AddInteger { value: 1 },
                line: 2,
                column: 1,
            },
            Op::Jump(1),
            Op::Halt,
        ],
        vec![ExprChunk {
            ops: vec![ExprOp::Load {
                dst: 0,
                slot: condition,
                column: 1,
            }],
            constants: Vec::new(),
            registers: 1,
            result: 0,
            line: 1,
        }],
        slots,
    );
    let ir = TypedIr::from_program(&program);
    assert!(
        ir.optimizations()
            .iter()
            .any(|optimization| matches!(optimization, IrOptimization::Constant { .. }))
    );
    assert!(
        ir.optimizations()
            .iter()
            .any(|optimization| matches!(optimization, IrOptimization::StrengthReduced { .. }))
    );
    assert!(
        ir.optimizations()
            .iter()
            .any(|optimization| matches!(optimization, IrOptimization::Hoisted { .. }))
    );
    assert!(ir.loops().iter().any(|loop_info| loop_info.header == 1));
}

#[test]
fn keeps_reachability_and_ssa_values_bounded_for_empty_programs() {
    let program = Program::from_chunks(Vec::new(), Vec::new(), SlotTable::new());
    let ir = TypedIr::from_program(&program);
    assert_eq!(ir.entry(), 0);
    assert_eq!(ir.blocks().len(), 1);
    assert!(ir.blocks()[0].reachable);
}

#[test]
fn handles_entry_back_edges_without_a_preheader() {
    let program = Program::from_chunks(vec![Op::Jump(0)], Vec::new(), SlotTable::new());
    let ir = TypedIr::from_program(&program);
    let loop_info = ir
        .loops()
        .iter()
        .find(|loop_info| loop_info.header == 0)
        .expect("entry back edge is a natural loop");
    assert_eq!(loop_info.preheader, None);
}

#[test]
fn keeps_a_self_loop_preheader_outside_the_loop() {
    let mut slots = SlotTable::new();
    let slot = slots.intern("slot");
    let program = Program::from_chunks(
        vec![
            Op::SetConst {
                slot,
                value: Value::Integer(1),
                line: 1,
            },
            Op::Jump(1),
        ],
        Vec::new(),
        slots,
    );
    let ir = TypedIr::from_program(&program);
    let loop_info = ir
        .loops()
        .iter()
        .find(|loop_info| loop_info.header == 1)
        .expect("self edge is a natural loop");
    assert_eq!(loop_info.blocks, vec![1]);
    assert_eq!(loop_info.preheader, Some(0));
}
