use crate::expr::compile_expression_into;
use velin_bytecode::{ChunkId, ExprChunk, ExprOp, Op, Pc, Program, ProgramArena, SlotTable};
use velin_eval::{Variables, evaluate};
use velin_syntax::{BinaryOp, Expr, Value};

/// Incrementally assembles a [`Program`].
///
/// The builder owns one [`SlotTable`] shared across every expression, so a
/// variable has the same slot everywhere in the program. Expressions are
/// compiled on demand and pooled; `Op`s refer to them by [`ChunkId`].
#[derive(Debug)]
pub struct ProgramBuilder {
    ops: Vec<Op>,
    pub(super) chunks: ProgramArena,
    slots: SlotTable,
    scratch: ExprChunk,
    known_constants: Vec<Option<Value>>,
    known_variables: Variables,
}

impl Default for ProgramBuilder {
    fn default() -> Self {
        Self {
            ops: Vec::new(),
            chunks: ProgramArena::new(),
            slots: SlotTable::default(),
            scratch: ExprChunk::new(0),
            known_constants: Vec::new(),
            known_variables: Variables::new(),
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
        let slot = self.slots.intern(name);
        if self.known_constants.len() <= slot as usize {
            self.known_constants.resize(slot as usize + 1, None);
        }
        slot
    }

    /// Compiles an expression and returns its pooled [`ChunkId`].
    ///
    /// # Panics
    /// Panics only if more than `u32::MAX` chunks are pooled, which the data
    /// budget makes unreachable.
    pub fn expr(&mut self, expression: &Expr, line: usize) -> ChunkId {
        compile_expression_into(expression, &mut self.slots, line, &mut self.scratch);
        self.sync_known_constants();
        self.append_scratch()
    }

    /// Compiles an assignment, selecting a direct constant or slot-copy op
    /// when the expression does not need the expression VM.
    pub fn set_op(&mut self, slot: u32, expression: &Expr, line: usize) -> Op {
        compile_expression_into(expression, &mut self.slots, line, &mut self.scratch);
        self.sync_known_constants();
        if let Some(value) = self.propagated_constant(expression, line) {
            return Op::SetConst {
                slot,
                value,
                line: self.scratch.line,
            };
        }
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
        self.sync_known_constants();
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
        self.chunks.append(&mut self.scratch)
    }

    /// Appends an op, returning its [`Pc`] (useful for patching jumps).
    ///
    /// # Panics
    /// Panics only if more than `u32::MAX` ops are pushed, which the data
    /// budget makes unreachable.
    pub fn push(&mut self, op: Op) -> Pc {
        let pc = u32::try_from(self.ops.len()).expect("pc fits in u32");
        self.track_constant_state(&op);
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
        self.chunks.finish(self.ops, self.slots)
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
        let chunk = self.chunks.chunk(id)?;
        let [ExprOp::Const(index)] = chunk.ops else {
            return None;
        };
        match chunk.constants.get(*index as usize)? {
            Value::Boolean(value) => Some(*value),
            _ => None,
        }
    }

    fn propagated_constant(&self, expression: &Expr, line: usize) -> Option<Value> {
        if self.known_variables.is_empty() {
            return None;
        }
        evaluate(expression, &self.known_variables, line).ok()
    }

    fn track_constant_state(&mut self, op: &Op) {
        match op {
            Op::SetConst { slot, value, .. } => self.set_known(*slot, Some(value.clone())),
            Op::CopySlot { slot, source, .. } => {
                let value = self
                    .known_constants
                    .get(*source as usize)
                    .cloned()
                    .flatten();
                self.set_known(*slot, value);
            }
            Op::Set { slot, .. } | Op::Update { slot, .. } => self.set_known(*slot, None),
            Op::Jump(_)
            | Op::JumpIfFalse { .. }
            | Op::JumpIfIntegerCompare { .. }
            | Op::Host(_)
            | Op::Halt => {
                self.known_constants.fill(None);
                self.known_variables.clear();
            }
        }
    }

    fn set_known(&mut self, slot: u32, value: Option<Value>) {
        if let Some(known) = self.known_constants.get_mut(slot as usize) {
            known.clone_from(&value);
            if let Some(name) = self.slots.name(slot) {
                match value {
                    Some(value) => {
                        self.known_variables.insert(name.to_owned(), value);
                    }
                    None => {
                        self.known_variables.remove(name);
                    }
                }
            }
        }
    }

    fn sync_known_constants(&mut self) {
        if self.known_constants.len() < self.slots.len() {
            self.known_constants.resize(self.slots.len(), None);
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
