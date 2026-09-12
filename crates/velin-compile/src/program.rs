//! Program-level bytecode: control flow over expression chunks.
//!
//! A [`Program`] is the unit the VM executes. It is a flat list of [`Op`]s
//! indexed by a program counter, plus the pool of [`ExprChunk`]s those ops
//! evaluate and the [`SlotTable`] describing the variable frame.
//!
//! This layer is deliberately generic. The only way to reach the outside world
//! is [`Op::Host`], an opaque effect the VM yields to the embedder.

use crate::bytecode::{ExprChunk, ExprChunkRef, ExprOp};
use crate::expr::compile_expression_into;
use crate::slots::SlotTable;
use serde::ser::{SerializeSeq, SerializeStruct};
use serde::{Deserialize, Serialize};
use std::ops::Range;
use velin_syntax::{BinaryOp, Builtin, DataMetrics, Expr, Value};

/// An index into [`Program::chunks`].
pub type ChunkId = u32;
/// A program-counter target: an index into [`Program::ops`].
pub type Pc = u32;

/// Ranges for one expression stored in [`Program`]'s contiguous arenas.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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

/// Precomputed properties used by the VM after a program is validated.
#[derive(Debug)]
pub struct ExecutionMetadata {
    chunks: Box<[ChunkExecutionMetadata]>,
    ops: Box<[OpExecutionMetadata]>,
    metrics: Box<[DataMetrics]>,
    quickened_calls: Box<[QuickenedCall]>,
    quickened_operands: Box<[QuickenedOperand]>,
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

impl ExecutionMetadata {
    pub(crate) fn new(program: &Program) -> Self {
        let mut metrics = Vec::new();
        let mut quickened_calls = Vec::new();
        let mut quickened_operands = Vec::new();
        let mut expression_heights = Vec::new();
        let chunks = (0..program.chunks.len())
            .map(|id| {
                let id = u32::try_from(id).expect("validated chunk id fits in u32");
                let chunk = program.chunk(id).expect("validated chunk range");
                let execution_data = match chunk.ops {
                    [ExprOp::Const(index)] => chunk.constants[*index as usize]
                        .data_metrics()
                        .ok()
                        .map_or(NO_METRICS, |value| push_metrics(&mut metrics, value)),
                    _ => quicken_call(
                        program,
                        id,
                        chunk,
                        &mut quickened_calls,
                        &mut quickened_operands,
                    )
                    .map_or(NO_METRICS, |index| QUICKENED_CALL_TAG | index),
                };
                let (max_stack, mutates_frame) =
                    expression_execution_shape(chunk.ops, &mut expression_heights);
                ChunkExecutionMetadata {
                    max_stack,
                    mutates_frame,
                    inherits_slot_metrics: matches!(chunk.ops, [ExprOp::Load { .. }]),
                    metrics: execution_data,
                }
            })
            .collect::<Vec<_>>()
            .into_boxed_slice();
        let ops = program
            .ops
            .iter()
            .map(|op| {
                let constant_metrics = match op {
                    Op::SetConst { value, .. } => value
                        .data_metrics()
                        .ok()
                        .map_or(NO_METRICS, |value| push_metrics(&mut metrics, value)),
                    _ => NO_METRICS,
                };
                OpExecutionMetadata {
                    line: op_line(program, op),
                    metrics: constant_metrics,
                }
            })
            .collect::<Vec<_>>()
            .into_boxed_slice();
        Self {
            chunks,
            ops,
            metrics: metrics.into_boxed_slice(),
            quickened_calls: quickened_calls.into_boxed_slice(),
            quickened_operands: quickened_operands.into_boxed_slice(),
        }
    }

    #[must_use]
    pub fn chunk(&self, id: ChunkId) -> Option<ChunkExecutionMetadata> {
        self.chunks.get(id as usize).copied()
    }

    #[must_use]
    pub fn chunk_constant_metrics(&self, id: ChunkId) -> Option<DataMetrics> {
        let metadata = self.chunk(id)?;
        self.constant_metrics(metadata)
    }

    #[must_use]
    pub fn chunk_with_constant_metrics(
        &self,
        id: ChunkId,
    ) -> Option<(ChunkExecutionMetadata, Option<DataMetrics>)> {
        let metadata = self.chunks.get(id as usize).copied()?;
        let metrics = self.constant_metrics(metadata);
        Some((metadata, metrics))
    }

    /// Returns execution properties, cached result metrics, and an optional
    /// quickened built-in call for one chunk.
    #[must_use]
    pub fn chunk_plan(
        &self,
        id: ChunkId,
    ) -> Option<(
        ChunkExecutionMetadata,
        Option<DataMetrics>,
        Option<QuickenedCallRef<'_>>,
    )> {
        let metadata = self.chunks.get(id as usize).copied()?;
        let metrics = self.constant_metrics(metadata);
        let quickened = self.quickened_call(metadata);
        Some((metadata, metrics, quickened))
    }

    #[must_use]
    pub fn op(&self, pc: usize) -> Option<OpExecutionMetadata> {
        self.ops.get(pc).copied()
    }

    #[must_use]
    pub fn op_constant_metrics(&self, pc: usize) -> Option<DataMetrics> {
        let metadata = self.op(pc)?;
        self.metrics.get(metadata.metrics as usize).copied()
    }

    fn constant_metrics(&self, metadata: ChunkExecutionMetadata) -> Option<DataMetrics> {
        if metadata.metrics & QUICKENED_CALL_TAG != 0 {
            return None;
        }
        self.metrics.get(metadata.metrics as usize).copied()
    }

    fn quickened_call(&self, metadata: ChunkExecutionMetadata) -> Option<QuickenedCallRef<'_>> {
        if metadata.metrics == NO_METRICS || metadata.metrics & QUICKENED_CALL_TAG == 0 {
            return None;
        }
        let index = (metadata.metrics & !QUICKENED_CALL_TAG) as usize;
        let call = self.quickened_calls.get(index)?;
        let operands = self
            .quickened_operands
            .get(call.operands.start as usize..call.operands.end as usize)?;
        Some(QuickenedCallRef {
            function: call.function,
            operands,
            line: call.line,
        })
    }
}

impl Program {
    /// Packs independently allocated expression chunks into contiguous arenas.
    #[must_use]
    pub fn from_chunks(ops: Vec<Op>, chunks: Vec<ExprChunk>, slots: SlotTable) -> Self {
        let mut packed = Vec::with_capacity(chunks.len());
        let op_count = chunks.iter().map(|chunk| chunk.ops.len()).sum();
        let constant_count = chunks.iter().map(|chunk| chunk.constants.len()).sum();
        let mut expr_ops = Vec::with_capacity(op_count);
        let mut constants = Vec::with_capacity(constant_count);
        for chunk in chunks {
            append_chunk(&mut packed, &mut expr_ops, &mut constants, chunk);
        }
        Self {
            ops,
            chunks: packed,
            expr_ops,
            constants,
            slots,
        }
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
}

impl Serialize for Program {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut state = serializer.serialize_struct("Program", 3)?;
        state.serialize_field("ops", &self.ops)?;
        state.serialize_field("chunks", &SerializedChunks(self))?;
        state.serialize_field("slots", &self.slots)?;
        state.end()
    }
}

struct SerializedChunks<'a>(&'a Program);

impl Serialize for SerializedChunks<'_> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut sequence = serializer.serialize_seq(Some(self.0.chunks.len()))?;
        for id in 0..self.0.chunks.len() {
            let id = u32::try_from(id).map_err(serde::ser::Error::custom)?;
            let chunk = self
                .0
                .chunk(id)
                .ok_or_else(|| serde::ser::Error::custom("invalid expression arena range"))?;
            sequence.serialize_element(&SerializedChunk(chunk))?;
        }
        sequence.end()
    }
}

struct SerializedChunk<'a>(ExprChunkRef<'a>);

impl Serialize for SerializedChunk<'_> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut state = serializer.serialize_struct("ExprChunk", 3)?;
        state.serialize_field("ops", self.0.ops)?;
        state.serialize_field("constants", self.0.constants)?;
        state.serialize_field("line", &self.0.line)?;
        state.end()
    }
}

// `SlotTable` is compile-time-only state, but programs are serialized for
// tooling/inspection, so it derives the same traits via a thin manual impl.
impl Serialize for SlotTable {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.names().serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for SlotTable {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let names = Vec::<String>::deserialize(deserializer)?;
        let mut table = SlotTable::new();
        for name in names {
            if table.get(&name).is_some() {
                return Err(serde::de::Error::custom(format!(
                    "duplicate slot name `{name}`"
                )));
            }
            table.intern(&name);
        }
        Ok(table)
    }
}

impl<'de> Deserialize<'de> for Program {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        struct ProgramWire {
            ops: Vec<Op>,
            chunks: Vec<ExprChunk>,
            slots: SlotTable,
        }

        let wire = ProgramWire::deserialize(deserializer)?;
        let program = Self::from_chunks(wire.ops, wire.chunks, wire.slots);
        program.validate().map_err(serde::de::Error::custom)?;
        Ok(program)
    }
}

/// Incrementally assembles a [`Program`].
///
/// The builder owns one [`SlotTable`] shared across every expression, so a
/// variable has the same slot everywhere in the program. Expressions are
/// compiled on demand and pooled; `Op`s refer to them by [`ChunkId`].
#[derive(Debug)]
pub struct ProgramBuilder {
    ops: Vec<Op>,
    chunks: Vec<ProgramChunk>,
    expr_ops: Vec<ExprOp>,
    constants: Vec<Value>,
    slots: SlotTable,
    scratch: ExprChunk,
}

impl Default for ProgramBuilder {
    fn default() -> Self {
        Self {
            ops: Vec::new(),
            chunks: Vec::new(),
            expr_ops: Vec::new(),
            constants: Vec::new(),
            slots: SlotTable::default(),
            scratch: ExprChunk::new(0),
        }
    }
}

impl ProgramBuilder {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Interns a variable name, returning its frame slot.
    pub fn slot(&mut self, name: &str) -> u32 {
        self.slots.intern(name)
    }

    /// Compiles an expression and returns its pooled [`ChunkId`].
    ///
    /// # Panics
    /// Panics only if more than `u32::MAX` chunks are pooled, which the data
    /// budget makes unreachable.
    pub fn expr(&mut self, expression: &Expr, line: usize) -> ChunkId {
        compile_expression_into(expression, &mut self.slots, line, &mut self.scratch);
        self.append_scratch()
    }

    /// Compiles an assignment, selecting a direct constant or slot-copy op
    /// when the expression does not need the expression VM.
    pub fn set_op(&mut self, slot: u32, expression: &Expr, line: usize) -> Op {
        compile_expression_into(expression, &mut self.slots, line, &mut self.scratch);
        match self.scratch.ops.as_slice() {
            [ExprOp::Const(index)] => Op::SetConst {
                slot,
                value: self.scratch.constants[*index as usize].clone(),
                line: self.scratch.line,
            },
            [
                ExprOp::Load {
                    slot: source,
                    column,
                },
            ] => Op::CopySlot {
                slot,
                source: *source,
                line: self.scratch.line,
                column: *column,
            },
            _ => {
                let value = self.append_scratch();
                Op::Set { slot, value }
            }
        }
    }

    /// Compiles a boolean guard, selecting a direct slot/integer comparison
    /// while retaining its expression chunk for diagnostics and analysis.
    pub fn jump_if_false_op(&mut self, expression: &Expr, line: usize, target: Pc) -> Op {
        compile_expression_into(expression, &mut self.slots, line, &mut self.scratch);
        let comparison = integer_comparison(&self.scratch);
        let condition = self.append_scratch();
        if let Some((slot, comparison, value)) = comparison {
            Op::JumpIfIntegerCompare {
                condition,
                slot,
                comparison,
                value,
                target,
            }
        } else {
            Op::JumpIfFalse { condition, target }
        }
    }

    fn append_scratch(&mut self) -> ChunkId {
        let id = u32::try_from(self.chunks.len()).expect("chunk id fits in u32");
        append_reusable_chunk(
            &mut self.chunks,
            &mut self.expr_ops,
            &mut self.constants,
            &mut self.scratch,
        );
        id
    }

    /// Appends an op, returning its [`Pc`] (useful for patching jumps).
    ///
    /// # Panics
    /// Panics only if more than `u32::MAX` ops are pushed, which the data
    /// budget makes unreachable.
    pub fn push(&mut self, op: Op) -> Pc {
        let pc = u32::try_from(self.ops.len()).expect("pc fits in u32");
        self.ops.push(op);
        pc
    }

    /// The program counter the next pushed op will occupy.
    ///
    /// # Panics
    /// Panics only if more than `u32::MAX` ops have been pushed, which the data
    /// budget makes unreachable.
    #[must_use]
    pub fn here(&self) -> Pc {
        u32::try_from(self.ops.len()).expect("pc fits in u32")
    }

    /// Overwrites a previously pushed op, used to back-patch jump targets.
    ///
    /// # Panics
    /// Panics if `pc` is out of range.
    pub fn patch(&mut self, pc: Pc, op: Op) {
        self.ops[pc as usize] = op;
    }

    /// Replaces the target of a conditional jump emitted with a placeholder.
    ///
    /// # Panics
    /// Panics if `pc` is out of range or does not contain a conditional jump.
    pub fn patch_condition_target(&mut self, pc: Pc, target: Pc) {
        match &mut self.ops[pc as usize] {
            Op::JumpIfFalse {
                target: current, ..
            }
            | Op::JumpIfIntegerCompare {
                target: current, ..
            } => *current = target,
            _ => panic!("conditional jump expected at pc {pc}"),
        }
    }

    /// Finishes the program, appending a trailing [`Op::Halt`] if the last op
    /// is not already a terminator.
    #[must_use]
    pub fn build(mut self) -> Program {
        if !matches!(self.ops.last(), Some(Op::Halt | Op::Jump(_))) {
            self.ops.push(Op::Halt);
        }
        self.optimize_control_flow();
        self.ops.shrink_to_fit();
        self.chunks.shrink_to_fit();
        self.expr_ops.shrink_to_fit();
        self.constants.shrink_to_fit();
        Program {
            ops: self.ops,
            chunks: self.chunks,
            expr_ops: self.expr_ops,
            constants: self.constants,
            slots: self.slots,
        }
    }

    fn optimize_control_flow(&mut self) {
        let mut has_jumps = false;
        for pc in 0..self.ops.len() {
            match self.ops[pc] {
                Op::JumpIfFalse { condition, target } => {
                    has_jumps = true;
                    if let Some(value) = self.constant_boolean(condition) {
                        self.ops[pc] = Op::Jump(if value {
                            u32::try_from(pc + 1).expect("pc fits in u32")
                        } else {
                            target
                        });
                    }
                }
                Op::Jump(_) | Op::JumpIfIntegerCompare { .. } => has_jumps = true,
                Op::Set { .. }
                | Op::SetConst { .. }
                | Op::CopySlot { .. }
                | Op::Update { .. }
                | Op::Host(_)
                | Op::Halt => {}
            }
        }
        if !has_jumps {
            return;
        }

        let targets: Vec<_> = self
            .ops
            .iter()
            .enumerate()
            .map(|(pc, op)| match op {
                Op::Jump(target)
                | Op::JumpIfFalse { target, .. }
                | Op::JumpIfIntegerCompare { target, .. } => {
                    resolve_jump_target(&self.ops, *target).unwrap_or(*target)
                }
                _ => u32::try_from(pc).expect("pc fits in u32"),
            })
            .collect();
        for (op, target) in self.ops.iter_mut().zip(targets) {
            match op {
                Op::Jump(current)
                | Op::JumpIfFalse {
                    target: current, ..
                }
                | Op::JumpIfIntegerCompare {
                    target: current, ..
                } => *current = target,
                _ => {}
            }
        }
    }

    fn constant_boolean(&self, id: ChunkId) -> Option<bool> {
        let chunk = self.chunks.get(id as usize)?;
        let [ExprOp::Const(index)] = self.expr_ops.get(range_usize(&chunk.ops))? else {
            return None;
        };
        let index = chunk.constants.start.checked_add(*index)?;
        match self.constants.get(index as usize)? {
            Value::Boolean(value) => Some(*value),
            _ => None,
        }
    }
}

fn resolve_jump_target(ops: &[Op], original: Pc) -> Option<Pc> {
    let mut target = original;
    for _ in 0..ops.len() {
        let Some(Op::Jump(next)) = ops.get(target as usize) else {
            return Some(target);
        };
        if *next == target {
            return None;
        }
        target = *next;
    }
    None
}

fn push_metrics(metrics: &mut Vec<DataMetrics>, value: DataMetrics) -> u32 {
    let index = u32::try_from(metrics.len()).expect("validated metadata count fits in u32");
    debug_assert!(index < QUICKENED_CALL_TAG);
    metrics.push(value);
    index
}

fn quicken_call(
    program: &Program,
    chunk_id: ChunkId,
    chunk: ExprChunkRef<'_>,
    calls: &mut Vec<QuickenedCall>,
    operands: &mut Vec<QuickenedOperand>,
) -> Option<u32> {
    let (last, argument_ops) = chunk.ops.split_last()?;
    let ExprOp::Call { function, argc } = last else {
        return None;
    };
    if argument_ops.len() != *argc as usize {
        return None;
    }

    let start = operands.len();
    let constants_start = program.chunks.get(chunk_id as usize)?.constants.start;
    for operand in argument_ops {
        let operand = match operand {
            ExprOp::Const(index) => {
                QuickenedOperand::Constant(constants_start.checked_add(*index)?)
            }
            ExprOp::Load { slot, .. } => QuickenedOperand::Slot(*slot),
            _ => {
                operands.truncate(start);
                return None;
            }
        };
        operands.push(operand);
    }
    let end = operands.len();
    let index = u32::try_from(calls.len()).expect("validated quickened call count fits in u32");
    debug_assert!(index < QUICKENED_CALL_TAG);
    calls.push(QuickenedCall {
        function: *function,
        operands: u32::try_from(start).expect("validated operand index fits in u32")
            ..u32::try_from(end).expect("validated operand index fits in u32"),
        line: chunk.line,
    });
    Some(index)
}

fn op_line(program: &Program, op: &Op) -> u32 {
    match op {
        Op::Set { value, .. }
        | Op::Update {
            operation: UpdateOp::Add { rhs: value },
            ..
        }
        | Op::Update {
            operation: UpdateOp::Push { value },
            ..
        }
        | Op::JumpIfFalse {
            condition: value, ..
        }
        | Op::JumpIfIntegerCompare {
            condition: value, ..
        } => program
            .chunks
            .get(*value as usize)
            .map_or(0, |chunk| chunk.line),
        Op::SetConst { line, .. } | Op::CopySlot { line, .. } | Op::Update { line, .. } => *line,
        Op::Host(host) => host.line,
        Op::Jump(_) | Op::Halt => 0,
    }
}

fn expression_execution_shape(ops: &[ExprOp], heights: &mut Vec<Option<usize>>) -> (u16, bool) {
    let mut has_branches = false;
    let mut mutates_frame = false;
    for op in ops {
        has_branches |= matches!(op, ExprOp::JumpIfFalse(_) | ExprOp::JumpIfTrue(_));
        mutates_frame |= matches!(op, ExprOp::Random { .. } | ExprOp::Chance { .. });
    }
    if !has_branches {
        let mut height = 0;
        let mut maximum = 0;
        for op in ops {
            maximum = maximum.max(height);
            height = expression_next_height(op, height);
        }
        maximum = maximum.max(height);
        return (
            u16::try_from(maximum).expect("validated expression stack height fits in u16"),
            mutates_frame,
        );
    }

    heights.clear();
    heights.resize(ops.len() + 1, None);
    heights[0] = Some(0usize);
    let mut maximum = 0;
    for pc in 0..ops.len() {
        let Some(height) = heights[pc] else {
            continue;
        };
        maximum = maximum.max(height);
        match &ops[pc] {
            ExprOp::JumpIfFalse(target) | ExprOp::JumpIfTrue(target) => {
                set_expression_height(&mut heights[*target as usize], height);
                set_expression_height(&mut heights[pc + 1], height);
            }
            op => set_expression_height(&mut heights[pc + 1], expression_next_height(op, height)),
        }
    }
    maximum = maximum.max(heights[ops.len()].unwrap_or(0));
    (
        u16::try_from(maximum).expect("validated expression stack height fits in u16"),
        mutates_frame,
    )
}

fn set_expression_height(slot: &mut Option<usize>, height: usize) {
    debug_assert!(slot.is_none_or(|existing| existing == height));
    *slot = Some(height);
}

fn expression_next_height(op: &ExprOp, height: usize) -> usize {
    match op {
        ExprOp::Const(_) | ExprOp::Load { .. } => height + 1,
        ExprOp::Unary(_) | ExprOp::Chance { .. } | ExprOp::AssertBoolean(_) => height,
        ExprOp::Binary(_) | ExprOp::Random { .. } => height - 1,
        ExprOp::Call { argc, .. } | ExprOp::Concat(argc) => height - *argc as usize + 1,
        ExprOp::JumpIfFalse(_) | ExprOp::JumpIfTrue(_) => unreachable!("handled as branches"),
    }
}

fn integer_comparison(chunk: &ExprChunk) -> Option<(u32, BinaryOp, i64)> {
    let [
        ExprOp::Load { slot, .. },
        ExprOp::Const(index),
        ExprOp::Binary(comparison),
    ] = chunk.ops.as_slice()
    else {
        return None;
    };
    if !matches!(
        comparison,
        BinaryOp::Equal
            | BinaryOp::NotEqual
            | BinaryOp::Less
            | BinaryOp::LessEqual
            | BinaryOp::Greater
            | BinaryOp::GreaterEqual
    ) {
        return None;
    }
    let Value::Integer(value) = chunk.constants.get(*index as usize)? else {
        return None;
    };
    Some((*slot, *comparison, *value))
}

fn append_chunk(
    chunks: &mut Vec<ProgramChunk>,
    expr_ops: &mut Vec<ExprOp>,
    constants: &mut Vec<Value>,
    chunk: ExprChunk,
) {
    let mut chunk = chunk;
    append_reusable_chunk(chunks, expr_ops, constants, &mut chunk);
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

#[cfg(test)]
mod tests {
    use super::*;
    use velin_syntax::{BinaryOp, Value};

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

        assert_eq!(metadata.chunk(chunk).unwrap().max_stack, 2);
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
    }

    #[test]
    fn validation_quickens_builtins_with_direct_operands() {
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
        let validated = crate::ValidatedProgram::new(builder.build()).unwrap();
        let metadata = validated.shared_execution_metadata();
        let (_, _, call) = metadata.chunk_plan(chunk).unwrap();
        let call = call.expect("simple built-in should be quickened");

        assert_eq!(call.function, Builtin::Contains);
        assert_eq!(
            call.operands,
            [QuickenedOperand::Slot(items), QuickenedOperand::Constant(0)]
        );
        assert_eq!(call.line, 4);
    }

    #[test]
    #[cfg(target_pointer_width = "64")]
    fn bytecode_layout_stays_compact() {
        assert_eq!(std::mem::size_of::<crate::ExprOp>(), 12);
        assert_eq!(std::mem::size_of::<Op>(), 32);
        assert_eq!(std::mem::size_of::<ProgramChunk>(), 20);
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
            Some(crate::ExprOp::Random { state_slot }) if *state_slot == rng_slot
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
}
