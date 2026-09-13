//! Typed basic-block IR used by execution preparation.
//!
//! The IR is deliberately conservative: host effects, random operations and
//! potentially failing operations are barriers. It provides stable block and
//! SSA-value identities for diagnostics and profile feedback without changing
//! the canonical bytecode semantics.

use crate::{Op, Program};
use std::collections::{BTreeSet, VecDeque};
use velin_syntax::{BinaryOp, UnaryOp, Value};

/// The runtime type domain carried by an IR value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IrType {
    Unknown,
    Integer,
    Boolean,
    String,
    Compound,
}

/// A monotonic SSA value identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct IrValue(pub u32);

/// One typed operation in a basic block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IrOp {
    Constant {
        dst: IrValue,
        value: Value,
        value_type: IrType,
    },
    Slot {
        dst: IrValue,
        slot: u32,
        value_type: IrType,
    },
    Unary {
        dst: IrValue,
        op: UnaryOp,
        source: IrValue,
        value_type: IrType,
    },
    Binary {
        dst: IrValue,
        left: IrValue,
        op: BinaryOp,
        right: IrValue,
        value_type: IrType,
    },
    Assign {
        slot: u32,
        value: IrValue,
    },
    Barrier,
}

/// Control-flow edge at the end of a basic block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IrTerminator {
    Fallthrough(u32),
    Jump(u32),
    Branch {
        condition: IrValue,
        if_true: u32,
        if_false: u32,
    },
    Halt,
}

/// One basic block with stable source program-counter boundaries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IrBlock {
    pub id: u32,
    pub start_pc: u32,
    pub operations: Vec<IrOp>,
    pub terminator: IrTerminator,
    pub reachable: bool,
}

/// A conservative typed SSA-style view of a validated program.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypedIr {
    blocks: Vec<IrBlock>,
    entry: u32,
    next_value: u32,
}

impl TypedIr {
    /// Builds block boundaries and typed assignments from a validated program.
    ///
    /// # Panics
    /// Panics if a validated program contains more operations or blocks than
    /// can be represented by a `u32` program counter or block identifier.
    #[must_use]
    pub fn from_program(program: &Program) -> Self {
        let mut boundaries = BTreeSet::from([0u32]);
        for (pc, op) in program.ops.iter().enumerate() {
            let pc = u32::try_from(pc).expect("validated pc fits in u32");
            match op {
                Op::Jump(target)
                | Op::JumpIfFalse { target, .. }
                | Op::JumpIfIntegerCompare { target, .. } => {
                    boundaries.insert(*target);
                    boundaries.insert(pc.saturating_add(1));
                }
                _ => {}
            }
        }
        let boundaries: Vec<u32> = boundaries
            .into_iter()
            .filter(|pc| usize::try_from(*pc).is_ok_and(|pc| pc < program.ops.len()))
            .collect();
        let mut block_for_pc = vec![0u32; program.ops.len()];
        for (id, window) in boundaries.windows(2).enumerate() {
            for pc in window[0]..window[1] {
                block_for_pc[pc as usize] = u32::try_from(id).expect("block id fits in u32");
            }
        }
        if let Some(start) = boundaries.last().copied() {
            let block_id = u32::try_from(boundaries.len() - 1).expect("block id fits");
            for block in block_for_pc.iter_mut().skip(start as usize) {
                *block = block_id;
            }
        }

        let mut next_value = 0;
        let mut blocks = Vec::with_capacity(boundaries.len());
        for (id, start) in boundaries.iter().copied().enumerate() {
            let end = boundaries
                .get(id + 1)
                .copied()
                .unwrap_or_else(|| u32::try_from(program.ops.len()).expect("program size fits"));
            let mut operations = Vec::new();
            for op in &program.ops[start as usize..end as usize] {
                lower_op(op, &mut operations, &mut next_value);
            }
            let last = program.ops.get(end.saturating_sub(1) as usize);
            let fallthrough = block_for_pc.get(end as usize).copied().unwrap_or(u32::MAX);
            let terminator = match last {
                Some(Op::Jump(target)) => IrTerminator::Jump(
                    block_for_pc
                        .get(*target as usize)
                        .copied()
                        .unwrap_or(u32::MAX),
                ),
                Some(Op::JumpIfFalse { target, .. } | Op::JumpIfIntegerCompare { target, .. }) => {
                    IrTerminator::Branch {
                        condition: IrValue(next_value.saturating_sub(1)),
                        if_true: fallthrough,
                        if_false: block_for_pc
                            .get(*target as usize)
                            .copied()
                            .unwrap_or(u32::MAX),
                    }
                }
                Some(Op::Halt) => IrTerminator::Halt,
                _ if end as usize >= program.ops.len() => IrTerminator::Halt,
                _ => IrTerminator::Fallthrough(block_for_pc[end as usize]),
            };
            blocks.push(IrBlock {
                id: u32::try_from(id).expect("block id fits in u32"),
                start_pc: start,
                operations,
                terminator,
                reachable: false,
            });
        }
        let mut ir = Self {
            blocks,
            entry: 0,
            next_value,
        };
        ir.mark_reachable();
        ir
    }

    /// Runs conservative SCCP-like cleanup: reachability is propagated only
    /// across CFG edges, while barrier operations are retained unchanged.
    pub fn optimize(&mut self) {
        self.mark_reachable();
    }

    #[must_use]
    pub fn blocks(&self) -> &[IrBlock] {
        &self.blocks
    }

    #[must_use]
    pub const fn entry(&self) -> u32 {
        self.entry
    }

    #[must_use]
    pub const fn value_count(&self) -> u32 {
        self.next_value
    }

    fn mark_reachable(&mut self) {
        for block in &mut self.blocks {
            block.reachable = false;
        }
        let mut pending = VecDeque::from([self.entry]);
        while let Some(id) = pending.pop_front() {
            let Some(block) = self.blocks.get_mut(id as usize) else {
                continue;
            };
            if block.reachable {
                continue;
            }
            block.reachable = true;
            let successors: Vec<u32> = match block.terminator {
                IrTerminator::Fallthrough(next) | IrTerminator::Jump(next) => vec![next],
                IrTerminator::Branch {
                    if_true, if_false, ..
                } => vec![if_true, if_false],
                IrTerminator::Halt => Vec::new(),
            };
            pending.extend(successors);
        }
    }
}

fn lower_op(op: &Op, operations: &mut Vec<IrOp>, next_value: &mut u32) {
    let new_value = |next_value: &mut u32| {
        let value = IrValue(*next_value);
        *next_value = (*next_value).saturating_add(1);
        value
    };
    match op {
        Op::SetConst { slot, value, .. } => {
            let dst = new_value(next_value);
            operations.push(IrOp::Constant {
                dst,
                value: value.clone(),
                value_type: value_type(value),
            });
            operations.push(IrOp::Assign {
                slot: *slot,
                value: dst,
            });
        }
        Op::CopySlot { slot, source, .. } => {
            let dst = new_value(next_value);
            operations.push(IrOp::Slot {
                dst,
                slot: *source,
                value_type: IrType::Unknown,
            });
            operations.push(IrOp::Assign {
                slot: *slot,
                value: dst,
            });
        }
        Op::Set { slot, .. } | Op::Update { slot, .. } => {
            operations.push(IrOp::Assign {
                slot: *slot,
                value: new_value(next_value),
            });
        }
        Op::Host(_) => operations.push(IrOp::Barrier),
        Op::Jump(_) | Op::JumpIfFalse { .. } | Op::JumpIfIntegerCompare { .. } | Op::Halt => {}
    }
}

fn value_type(value: &Value) -> IrType {
    match value {
        Value::Integer(_) => IrType::Integer,
        Value::Boolean(_) => IrType::Boolean,
        Value::String(_) => IrType::String,
        Value::List(_) | Value::Record(_) => IrType::Compound,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ExprChunk, ExprOp, SlotTable};

    #[test]
    fn builds_reachable_typed_blocks_and_ssa_values() {
        let mut slots = SlotTable::new();
        let value = slots.intern("value");
        let program = Program::from_chunks(
            vec![
                Op::SetConst {
                    slot: value,
                    value: Value::Integer(1),
                    line: 1,
                },
                Op::Jump(3),
                Op::SetConst {
                    slot: value,
                    value: Value::Integer(2),
                    line: 2,
                },
                Op::Halt,
            ],
            vec![ExprChunk {
                ops: vec![ExprOp::Const {
                    dst: 0,
                    constant: 0,
                }],
                constants: vec![Value::Integer(0)],
                registers: 1,
                result: 0,
                line: 1,
            }],
            slots,
        );
        let mut ir = TypedIr::from_program(&program);
        ir.optimize();
        assert!(ir.blocks().iter().any(|block| block.reachable));
        assert!(ir.value_count() >= 2);
        assert_eq!(ir.entry(), 0);
    }
}
