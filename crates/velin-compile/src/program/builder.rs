use super::{
    BinaryOp, ChunkId, Expr, ExprChunk, ExprOp, Op, Pc, Program, ProgramChunk, SlotTable, Value,
    append_reusable_chunk, compile_expression_into, range_usize,
};

/// Incrementally assembles a [`Program`].
///
/// The builder owns one [`SlotTable`] shared across every expression, so a
/// variable has the same slot everywhere in the program. Expressions are
/// compiled on demand and pooled; `Op`s refer to them by [`ChunkId`].
#[derive(Debug)]
pub struct ProgramBuilder {
    ops: Vec<Op>,
    pub(super) chunks: Vec<ProgramChunk>,
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
