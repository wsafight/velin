use super::{
    BinaryOp, ChunkExecutionMetadata, ChunkId, DataMetrics, ExecutionMetadata, ExprChunkRef,
    ExprOp, NO_METRICS, Op, OpExecutionMetadata, PreparedExpr, Program, QUICKENED_CALL_TAG,
    QuickenedCall, QuickenedCallRef, QuickenedOperand, UpdateOp, Value,
};

impl ExecutionMetadata {
    pub(crate) fn new(program: &Program) -> Self {
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
        let mut quickened_calls = Vec::new();
        let mut quickened_operands = Vec::new();
        let mut prepared = Vec::new();
        let mut expression_heights = Vec::new();
        let chunks = (0..program.chunks.len())
            .map(|id| {
                let id = u32::try_from(id).expect("validated chunk id fits in u32");
                let chunk = program.chunk(id).expect("validated chunk range");
                let constant_start = program.chunks[id as usize].constants.start;
                let execution_data = match chunk.ops {
                    [ExprOp::Const(index)] => constant_start
                        .checked_add(*index)
                        .and_then(|index| program_constant_metrics.get(index as usize).copied())
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
                prepared.push(prepare_expression(chunk));
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
            program_constant_metrics,
            quickened_calls: quickened_calls.into_boxed_slice(),
            quickened_operands: quickened_operands.into_boxed_slice(),
            prepared: prepared.into_boxed_slice(),
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

    /// Returns a small non-serialized expression plan when one was prepared.
    #[must_use]
    pub fn prepared_expr(&self, id: ChunkId) -> Option<PreparedExpr> {
        self.prepared.get(id as usize).copied().flatten()
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

fn prepare_expression(chunk: ExprChunkRef<'_>) -> Option<PreparedExpr> {
    match chunk.ops {
        [ExprOp::Const(constant)] if (*constant as usize) < chunk.constants.len() => {
            Some(PreparedExpr::Constant {
                constant: *constant,
            })
        }
        [ExprOp::Load { slot, .. }] => Some(PreparedExpr::Load { slot: *slot }),
        [
            ExprOp::Load { slot, .. },
            ExprOp::Const(constant),
            ExprOp::Binary(operation),
        ] if matches!(
            chunk.constants.get(*constant as usize),
            Some(Value::Integer(_))
        ) && !matches!(operation, BinaryOp::And | BinaryOp::Or) =>
        {
            let Value::Integer(value) = chunk.constants[*constant as usize] else {
                unreachable!("integer constant was matched")
            };
            Some(PreparedExpr::IntegerBinaryLiteral {
                slot: *slot,
                operation: *operation,
                value,
            })
        }
        _ => None,
    }
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
