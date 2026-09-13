use super::{
    ChunkExecutionMetadata, ChunkId, DataMetrics, ExecutionMetadata, ExprOp, NO_METRICS, Op,
    OpExecutionMetadata, Program, TypedIr, UpdateOp,
};

impl ExecutionMetadata {
    pub(crate) fn new(program: &Program) -> Self {
        let mut typed_ir = TypedIr::from_program(program);
        typed_ir.optimize();
        let mut metrics = Vec::new();
        let program_constant_metrics = program
            .constants
            .iter()
            .map(|value| {
                value
                    .data_metrics()
                    .expect("validated program constant metrics")
            })
            .collect::<Vec<_>>()
            .into_boxed_slice();
        let chunks = (0..program.chunks.len())
            .map(|id| {
                let id = u32::try_from(id).expect("validated chunk id fits in u32");
                let chunk = program.chunk(id).expect("validated chunk range");
                let constant_start = program.chunks[id as usize].constants.start;
                let result_metrics = match chunk.ops {
                    [ExprOp::Const { dst, constant }] if *dst == chunk.result => constant_start
                        .checked_add(*constant)
                        .and_then(|index| program_constant_metrics.get(index as usize).copied()),
                    _ => None,
                };
                ChunkExecutionMetadata {
                    registers: chunk.registers,
                    mutates_frame: chunk
                        .ops
                        .iter()
                        .any(|op| matches!(op, ExprOp::Random { .. } | ExprOp::Chance { .. })),
                    inherits_slot_metrics: matches!(
                        chunk.ops,
                        [ExprOp::Load { dst, .. }] if *dst == chunk.result
                    ),
                    metrics: result_metrics
                        .map_or(NO_METRICS, |value| push_metrics(&mut metrics, value)),
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
            typed_ir,
            chunks,
            ops,
            metrics: metrics.into_boxed_slice(),
            program_constant_metrics,
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

    /// Returns the conservative typed basic-block/SSA view built once for the
    /// validated program.
    #[must_use]
    pub fn typed_ir(&self) -> &TypedIr {
        &self.typed_ir
    }

    /// Returns metrics for constants in the program-wide constant arena.
    #[must_use]
    pub fn program_constant_metrics(&self) -> &[DataMetrics] {
        &self.program_constant_metrics
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
        self.metrics.get(metadata.metrics as usize).copied()
    }
}

fn push_metrics(metrics: &mut Vec<DataMetrics>, value: DataMetrics) -> u32 {
    let index = u32::try_from(metrics.len()).expect("validated metadata count fits in u32");
    metrics.push(value);
    index
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
