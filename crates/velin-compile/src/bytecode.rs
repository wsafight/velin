//! The flat bytecode the compiler emits and the VM executes.
//!
//! There are two layers, mirroring the two layers of the source language:
//!
//! * [`ExprChunk`] — a stack machine for a single expression. It replaces the
//!   recursive `Expr` walk with a flat `Vec<ExprOp>`, so evaluation is a tight
//!   loop over a slice with no pointer chasing.
//! * [`Op`] — program-level control flow (see [`crate::program`]). Each control
//!   op refers to expression chunks by index.
//!
//! Neither layer contains host-domain concepts. Host effects are represented
//! by the opaque [`Op::Host`] opcode, which the VM hands back to the embedder
//! without interpreting.

use serde::{Deserialize, Serialize};
use velin_syntax::{BinaryOp, Builtin, UnaryOp, Value};

/// A single stack-machine instruction for evaluating one expression.
///
/// Operands are pushed onto an operand stack; each op consumes its inputs from
/// the top of the stack and pushes its result. A well-formed chunk always
/// leaves exactly one value on the stack.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExprOp {
    /// Push a constant from the chunk's constant pool.
    Const(u32),
    /// Push the current value of a frame slot, retaining its source column for
    /// definite-assignment diagnostics.
    Load { slot: u32, column: u32 },
    /// Apply a unary operator to the top of the stack.
    Unary(UnaryOp),
    /// Apply a binary operator to the top two stack values.
    ///
    /// `and`/`or` are *not* emitted as `Binary`; they compile to
    /// short-circuiting [`ExprOp::JumpIfFalse`] / [`ExprOp::JumpIfTrue`] so an
    /// unassigned right-hand operand is never evaluated (matching the
    /// tree-walker).
    Binary(BinaryOp),
    /// Call a built-in with `argc` values taken from the stack.
    Call { function: Builtin, argc: u32 },
    /// Draw an integer from the inclusive bounds on the stack, reading and
    /// writing the deterministic RNG state in `state_slot`.
    Random { state_slot: u32 },
    /// Draw a boolean using the integer percentage on the stack, reading and
    /// writing the deterministic RNG state in `state_slot`.
    Chance { state_slot: u32 },
    /// If the top of the stack is boolean `false`, leave it and jump to the
    /// target op index; otherwise leave it and continue. Used for `and`; the
    /// fallthrough path evaluates the right operand and combines both values.
    JumpIfFalse(u32),
    /// If the top of the stack is boolean `true`, leave it and jump to the
    /// target op index; otherwise leave it and continue. Used for `or`.
    JumpIfTrue(u32),
    /// Require the top stack value to be boolean without consuming it.
    /// Emitted after the right operand of `and`/`or`, whose value becomes the
    /// expression result when the left operand does not short-circuit.
    AssertBoolean(BinaryOp),
    /// Concatenate the top `count` stack values into one string, rendering each
    /// with [`Value::to_display`]. Emitted for string interpolation; literal
    /// segments are pushed as string constants and holes as arbitrary values,
    /// so a uniform display-then-join produces the interpolated text.
    Concat(u32),
}

/// A compiled expression: a constant pool plus a flat op stream.
///
/// `line` is the 1-based source line, carried for error reporting only.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExprChunk {
    pub ops: Vec<ExprOp>,
    pub constants: Vec<Value>,
    pub line: u32,
}

/// Borrowed expression bytecode, either from a standalone [`ExprChunk`] or a
/// packed program arena.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExprChunkRef<'a> {
    pub ops: &'a [ExprOp],
    pub constants: &'a [Value],
    pub line: u32,
}

impl ExprChunk {
    #[must_use]
    pub fn new(line: usize) -> Self {
        Self {
            ops: Vec::new(),
            constants: Vec::new(),
            line: crate::compact_source_position(line),
        }
    }

    /// Interns a constant, returning its pool index (deduplicating equal
    /// values so repeated literals share one slot).
    ///
    /// # Panics
    /// Panics only if more than `u32::MAX` constants are interned, which the
    /// data budget makes unreachable.
    pub fn constant(&mut self, value: Value) -> u32 {
        if let Some(index) = self
            .constants
            .iter()
            .position(|existing| *existing == value)
        {
            return u32::try_from(index).expect("constant index fits in u32");
        }
        let index = u32::try_from(self.constants.len()).expect("constant index fits in u32");
        self.constants.push(value);
        index
    }

    /// Appends an op and returns its index (useful for patching jumps).
    pub fn push(&mut self, op: ExprOp) -> usize {
        self.ops.push(op);
        self.ops.len() - 1
    }

    /// Borrows this standalone chunk in the same form used by packed programs.
    #[must_use]
    pub fn as_chunk_ref(&self) -> ExprChunkRef<'_> {
        ExprChunkRef {
            ops: &self.ops,
            constants: &self.constants,
            line: self.line,
        }
    }

    pub(crate) fn compact(&mut self) {
        self.ops.shrink_to_fit();
        self.constants.shrink_to_fit();
    }
}
