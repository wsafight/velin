//! Program-level bytecode: control flow over expression chunks.
//!
//! A [`Program`] is the unit the VM executes. It is a flat list of [`Op`]s
//! indexed by a program counter, plus the pool of [`ExprChunk`]s those ops
//! evaluate and the [`SlotTable`] describing the variable frame.
//!
//! This layer is deliberately generic. The only way to reach the outside world
//! is [`Op::Host`], an opaque effect the VM yields to the embedder.

use crate::bytecode::{ExprChunk, ExprChunkRef, ExprOp};
use crate::slots::SlotTable;
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};
use std::ops::Range;
use velin_syntax::{BinaryOp, Builtin, DataMetrics, UnaryOp, Value};

mod metadata;
#[cfg(feature = "serde")]
mod wire;

/// An index into [`Program::chunks`].
pub type ChunkId = u32;
/// A program-counter target: an index into [`Program::ops`].
pub type Pc = u32;

/// Ranges for one expression stored in [`Program`]'s contiguous arenas.
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProgramChunk {
    pub ops: Range<u32>,
    pub constants: Range<u32>,
    pub line: u32,
}

/// An ownership-aware update of a slot from its current value.
///
/// Surface lowering emits these only for expressions whose first operand is
/// the assignment target itself, such as `items = push(items, value)`. Keeping
/// the remaining operands as chunks lets the VM evaluate them before taking
/// ownership of the destination, so failures leave the old value untouched.
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpdateOp {
    Add { rhs: ChunkId },
    AddInteger { value: i64 },
    Push { value: ChunkId },
    Put { key: ChunkId, value: ChunkId },
    Remove { key: ChunkId },
}

/// Cold payload for a host yield.
///
/// Host instructions are comparatively rare and stop the VM. Keeping their
/// variable-sized payload behind a pointer prevents every hot control-flow
/// instruction from being sized like a host call.
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostOp {
    pub host_id: u32,
    pub args: Box<[ChunkId]>,
    /// Destination for a required host return value, or `None` for a
    /// side-effect-only command.
    pub bind: Option<u32>,
    /// Source line used when a bound command resumes without a value.
    pub line: u32,
}

impl HostOp {
    /// Creates a host payload and freezes its argument list to exact capacity.
    #[must_use]
    pub fn new(
        host_id: u32,
        args: impl Into<Box<[ChunkId]>>,
        bind: Option<u32>,
        line: usize,
    ) -> Self {
        Self {
            host_id,
            args: args.into(),
            bind,
            line: crate::compact_source_position(line),
        }
    }
}

/// A control-flow instruction.
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Op {
    /// Evaluate a chunk and store the result into a frame slot.
    Set { slot: u32, value: ChunkId },
    /// Store a compile-time constant without entering the expression VM.
    SetConst { slot: u32, value: Value, line: u32 },
    /// Copy an assigned frame slot without entering the expression VM.
    CopySlot {
        slot: u32,
        source: u32,
        line: u32,
        column: u32,
    },
    /// Update `slot` using its current value without creating a temporary copy
    /// when the value has no real aliases.
    Update {
        slot: u32,
        operation: UpdateOp,
        line: u32,
        column: u32,
    },
    /// Unconditional jump to a program counter.
    Jump(Pc),
    /// Evaluate a chunk; if it is boolean `false`, jump to the target.
    JumpIfFalse { condition: ChunkId, target: Pc },
    /// Compare an integer slot with a literal; jump when the result is false.
    /// `condition` retains the original chunk for source-aware static analysis.
    JumpIfIntegerCompare {
        condition: ChunkId,
        slot: u32,
        comparison: BinaryOp,
        value: i64,
        target: Pc,
    },
    /// Yield an opaque host effect: evaluate `args` and hand `(host_id,
    /// values)` back to the embedder. Execution resumes at the next op when the
    /// host calls `resume`; a `bind` destination makes its return value required.
    Host(Box<HostOp>),
    /// Halt execution successfully.
    Halt,
}

impl Op {
    /// Creates an ownership-aware update with a compact source position.
    #[must_use]
    pub fn update(slot: u32, operation: UpdateOp, line: usize, column: usize) -> Self {
        Self::Update {
            slot,
            operation,
            line: crate::compact_source_position(line),
            column: crate::compact_source_position(column),
        }
    }

    /// Creates a boxed host instruction so its cold payload does not widen
    /// every instruction in the control-flow stream.
    #[must_use]
    pub fn host(
        host_id: u32,
        args: impl Into<Box<[ChunkId]>>,
        bind: Option<u32>,
        line: usize,
    ) -> Self {
        Self::Host(Box::new(HostOp::new(host_id, args, bind, line)))
    }
}

/// A compiled, executable program.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Program {
    pub ops: Vec<Op>,
    pub chunks: Vec<ProgramChunk>,
    pub expr_ops: Vec<ExprOp>,
    pub constants: Vec<Value>,
    pub slots: SlotTable,
}

/// Storage for the packed expression arenas assembled by the compiler.
///
/// The arena keeps expression instructions and constants contiguous while the
/// compiler is building a program. It contains no source-language or evaluator
/// logic, so it belongs with the bytecode representation rather than the
/// compiler's lowering code.
#[derive(Debug, Default)]
pub struct ProgramArena {
    chunks: Vec<ProgramChunk>,
    expr_ops: Vec<ExprOp>,
    constants: Vec<Value>,
}

impl ProgramArena {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Appends a reusable chunk and returns its stable arena id.
    ///
    /// # Panics
    /// Panics only if more than `u32::MAX` chunks are appended, which the
    /// validated program limits make unreachable in practice.
    pub fn append(&mut self, chunk: &mut ExprChunk) -> ChunkId {
        let id = u32::try_from(self.chunks.len()).expect("chunk id fits in u32");
        append_reusable_chunk(
            &mut self.chunks,
            &mut self.expr_ops,
            &mut self.constants,
            chunk,
        );
        id
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.chunks.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.chunks.is_empty()
    }

    /// Borrows one expression from the packed arenas.
    #[must_use]
    pub fn chunk(&self, id: ChunkId) -> Option<ExprChunkRef<'_>> {
        let chunk = self.chunks.get(id as usize)?;
        let ops = self.expr_ops.get(range_usize(&chunk.ops))?;
        let constants = self.constants.get(range_usize(&chunk.constants))?;
        Some(ExprChunkRef {
            ops,
            constants,
            line: chunk.line,
        })
    }

    /// Finishes the arena without copying its packed storage into a program.
    #[must_use]
    pub fn finish(mut self, ops: Vec<Op>, slots: SlotTable) -> Program {
        self.chunks.shrink_to_fit();
        self.expr_ops.shrink_to_fit();
        self.constants.shrink_to_fit();
        Program {
            ops,
            chunks: self.chunks,
            expr_ops: self.expr_ops,
            constants: self.constants,
            slots,
        }
    }
}

/// Precomputed properties used by the VM after a program is validated.
#[derive(Debug)]
pub struct ExecutionMetadata {
    chunks: Box<[ChunkExecutionMetadata]>,
    ops: Box<[OpExecutionMetadata]>,
    metrics: Box<[DataMetrics]>,
    program_constant_metrics: Box<[DataMetrics]>,
    quickened_calls: Box<[QuickenedCall]>,
    quickened_operands: Box<[QuickenedOperand]>,
    prepared: Box<[u32]>,
    prepared_values: Box<[PreparedExpr]>,
    registers: Box<[u32]>,
    register_values: Box<[RegisterExpr]>,
}

#[derive(Debug)]
struct QuickenedCall {
    function: Builtin,
    operands: Range<u32>,
    line: u32,
}

/// One directly addressable operand of a quickened built-in call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuickenedOperand {
    Constant(u32),
    Slot(u32),
}

/// A built-in call whose operands were resolved during execution preparation.
#[derive(Debug, Clone, Copy)]
pub struct QuickenedCallRef<'a> {
    pub function: Builtin,
    pub operands: &'a [QuickenedOperand],
    pub line: u32,
}

/// A non-serialized execution plan for a small straight-line expression.
///
/// The canonical [`ExprOp`] sequence remains the compatibility format. This
/// plan is rebuilt after validation and only removes stack bookkeeping for
/// direct operands; it never contains alternate operator semantics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreparedExpr {
    Constant {
        constant: u32,
    },
    Load {
        slot: u32,
    },
    IntegerBinaryLiteral {
        slot: u32,
        operation: BinaryOp,
        value: i64,
    },
}

/// A non-serialized SSA-style value plan with compact physical registers for a
/// long, straight-line expression.
///
/// The canonical [`ExprOp`] sequence remains the source of truth and is kept
/// as the fallback for short or control-flow-heavy expressions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegisterExpr {
    pub ops: Box<[RegisterOp]>,
    pub result: u16,
    pub registers: u16,
}

/// Type information inferred for a temporary in the scalar execution plan.
///
/// This is an execution hint, not a replacement for runtime checks. Unknown
/// values and externally assembled programs always retain the general
/// operator path when the hint cannot prove a narrower type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegisterType {
    Unknown,
    Integer,
    Boolean,
    String,
    Compound,
}

/// One typed operation in a [`RegisterExpr`] execution plan. The logical value
/// flow is SSA-style; consumed source registers may be reused as destinations
/// by the compact linear-scan allocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegisterOp {
    LoadConstant {
        dst: u16,
        constant: u32,
        result_type: RegisterType,
    },
    LoadSlot {
        dst: u16,
        slot: u32,
        result_type: RegisterType,
    },
    Unary {
        dst: u16,
        op: UnaryOp,
        source: u16,
        result_type: RegisterType,
    },
    Binary {
        dst: u16,
        left: u16,
        op: BinaryOp,
        right: u16,
        result_type: RegisterType,
    },
}

/// Execution properties of one expression chunk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChunkExecutionMetadata {
    pub max_stack: u16,
    pub mutates_frame: bool,
    pub inherits_slot_metrics: bool,
    metrics: u32,
}

/// Execution properties of one program instruction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OpExecutionMetadata {
    pub line: u32,
    metrics: u32,
}

const NO_METRICS: u32 = u32::MAX;
const QUICKENED_CALL_TAG: u32 = 1 << 31;
const NO_PLAN: u32 = u32::MAX;

impl Program {
    /// Packs independently allocated expression chunks into contiguous arenas.
    #[must_use]
    pub fn from_chunks(ops: Vec<Op>, chunks: Vec<ExprChunk>, slots: SlotTable) -> Self {
        let mut arena = ProgramArena {
            chunks: Vec::with_capacity(chunks.len()),
            expr_ops: Vec::with_capacity(chunks.iter().map(|chunk| chunk.ops.len()).sum()),
            constants: Vec::with_capacity(chunks.iter().map(|chunk| chunk.constants.len()).sum()),
        };

        for chunk in chunks {
            let mut chunk = chunk;
            arena.append(&mut chunk);
        }
        arena.finish(ops, slots)
    }

    /// Borrows one expression from the packed arenas.
    #[must_use]
    pub fn chunk(&self, id: ChunkId) -> Option<ExprChunkRef<'_>> {
        let chunk = self.chunks.get(id as usize)?;
        let ops = self.expr_ops.get(range_usize(&chunk.ops))?;
        let constants = self.constants.get(range_usize(&chunk.constants))?;
        Some(ExprChunkRef {
            ops,
            constants,
            line: chunk.line,
        })
    }

    /// Removes slot names and reverse lookup for numeric-slot-only hosts.
    ///
    /// Slot IDs, frame width, bytecode, and the internal RNG slot remain
    /// unchanged. The returned program is intended for execution and cannot
    /// be used for name-based variable binding or source-friendly diagnostics.
    #[must_use]
    pub fn without_slot_names(mut self) -> Self {
        self.slots = self.slots.without_names();
        self
    }
}

fn append_reusable_chunk(
    chunks: &mut Vec<ProgramChunk>,
    expr_ops: &mut Vec<ExprOp>,
    constants: &mut Vec<Value>,
    chunk: &mut ExprChunk,
) {
    let op_start = u32::try_from(expr_ops.len()).expect("expression arena fits in u32");
    expr_ops.append(&mut chunk.ops);
    let op_end = u32::try_from(expr_ops.len()).expect("expression arena fits in u32");
    let constant_start = u32::try_from(constants.len()).expect("constant arena fits in u32");
    constants.append(&mut chunk.constants);
    let constant_end = u32::try_from(constants.len()).expect("constant arena fits in u32");
    chunks.push(ProgramChunk {
        ops: op_start..op_end,
        constants: constant_start..constant_end,
        line: chunk.line,
    });
}

fn range_usize(range: &Range<u32>) -> Range<usize> {
    range.start as usize..range.end as usize
}
