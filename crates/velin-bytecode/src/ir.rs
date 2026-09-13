//! Typed basic-block IR used by execution preparation.
//!
//! The IR is deliberately conservative: host effects, random operations and
//! potentially failing operations are barriers. It provides a real SSA view
//! for control-flow joins and performs only transformations whose safety is
//! independent of the canonical bytecode representation.

mod optimize;
mod ssa;

use crate::{ChunkId, Program, UpdateOp};
use std::collections::BTreeSet;
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
    /// A merge value with one incoming definition for each predecessor that
    /// assigned the slot. Missing inputs represent an unassigned path.
    Phi {
        dst: IrValue,
        slot: u32,
        inputs: Vec<(u32, IrValue)>,
        value_type: IrType,
    },
    Constant {
        dst: IrValue,
        value: Value,
        value_type: IrType,
    },
    Slot {
        dst: IrValue,
        slot: u32,
        source: Option<IrValue>,
        value_type: IrType,
    },
    /// An expression chunk evaluated by the canonical register VM.
    Evaluate {
        dst: IrValue,
        chunk: ChunkId,
        constant: Option<Value>,
        value_type: IrType,
        barrier: bool,
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
    Update {
        dst: IrValue,
        slot: u32,
        source: Option<IrValue>,
        operation: UpdateOp,
    },
    /// A checked integer induction update recognized inside a loop.
    Induction {
        dst: IrValue,
        slot: u32,
        source: Option<IrValue>,
        step: i64,
    },
    /// A host result or another value whose definition is external to the IR.
    Unknown {
        dst: IrValue,
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

/// A natural loop discovered from a back edge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IrLoop {
    pub header: u32,
    pub blocks: Vec<u32>,
    pub back_edges: Vec<(u32, u32)>,
    pub preheader: Option<u32>,
}

/// A safe transformation recorded by the internal optimizer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IrOptimization {
    Constant {
        value: IrValue,
        constant: Value,
    },
    BranchFolded {
        block: u32,
        target: u32,
    },
    Hoisted {
        loop_header: u32,
        from_block: u32,
        value: IrValue,
    },
    StrengthReduced {
        loop_header: u32,
        slot: u32,
        step: i64,
    },
}

/// One basic block with stable source program-counter boundaries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IrBlock {
    pub id: u32,
    pub start_pc: u32,
    pub operations: Vec<IrOp>,
    pub terminator: IrTerminator,
    pub predecessors: Vec<u32>,
    pub loop_depth: u16,
    pub reachable: bool,
}

/// A typed SSA view and conservative optimizer for a validated program.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypedIr {
    blocks: Vec<IrBlock>,
    entry: u32,
    next_value: u32,
    constants: Vec<Option<Value>>,
    loops: Vec<IrLoop>,
    optimizations: Vec<IrOptimization>,
}

impl TypedIr {
    /// Builds basic blocks, SSA definitions, and merge phis from a validated
    /// program.
    #[must_use]
    pub fn from_program(program: &Program) -> Self {
        let (ranges, block_for_pc) = ssa::block_ranges(program);
        let skeletons = ranges
            .iter()
            .map(|(start, end)| ssa::skeleton_terminator(program, *start, *end, &block_for_pc))
            .collect::<Vec<_>>();
        let predecessors = ssa::predecessor_lists(&skeletons);
        let mut phi_slots = vec![BTreeSet::new(); ranges.len()];
        let mut pass = ssa::run_ssa_pass(program, &ranges, &skeletons, &predecessors, &phi_slots);
        for _ in 0..ranges.len().saturating_add(1) {
            if pass.needed_phis.is_empty() {
                break;
            }
            let mut changed = false;
            for (block, slot) in pass.needed_phis.drain(..) {
                changed |= phi_slots[block as usize].insert(slot);
            }
            if !changed {
                break;
            }
            pass = ssa::run_ssa_pass(program, &ranges, &skeletons, &predecessors, &phi_slots);
        }

        let mut ir = Self {
            blocks: pass.blocks,
            entry: 0,
            next_value: pass.next_value,
            constants: vec![None; pass.next_value as usize],
            loops: Vec::new(),
            optimizations: Vec::new(),
        };
        ssa::fill_phi_inputs(&mut ir.blocks, &predecessors, &pass.out_states);
        ir.optimize();
        ir
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

    /// Returns the compile-time constant known for an SSA value, if any.
    #[must_use]
    pub fn constant(&self, value: IrValue) -> Option<&Value> {
        self.constants.get(value.0 as usize)?.as_ref()
    }

    #[must_use]
    pub fn loops(&self) -> &[IrLoop] {
        &self.loops
    }

    #[must_use]
    pub fn optimizations(&self) -> &[IrOptimization] {
        &self.optimizations
    }
}

pub(super) fn operation_value(operation: &IrOp) -> Option<IrValue> {
    match operation {
        IrOp::Phi { dst, .. }
        | IrOp::Constant { dst, .. }
        | IrOp::Slot { dst, .. }
        | IrOp::Evaluate { dst, .. }
        | IrOp::Unary { dst, .. }
        | IrOp::Binary { dst, .. }
        | IrOp::Update { dst, .. }
        | IrOp::Induction { dst, .. }
        | IrOp::Unknown { dst } => Some(*dst),
        IrOp::Assign { .. } | IrOp::Barrier => None,
    }
}

#[cfg(test)]
#[path = "ir_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "ir_coverage_tests.rs"]
mod coverage_tests;
