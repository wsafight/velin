//! Register bytecode emitted by the compiler and executed by the VM.
//!
//! Expressions use explicit source and destination registers. Short-circuit
//! branches retain their condition register, while built-ins and interpolation
//! consume contiguous register ranges. Program-level control flow remains in
//! [`crate::Op`] and refers to expression chunks by index.

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};
use std::ops::Range;
use velin_syntax::{BinaryOp, Builtin, UnaryOp, Value};

/// A physical register within one expression chunk.
pub type Register = u16;

/// One register instruction in an expression chunk.
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExprOp {
    /// Copy a value from the chunk constant pool into `dst`.
    Const { dst: Register, constant: u32 },
    /// Copy the current value of a frame slot into `dst`.
    Load {
        dst: Register,
        slot: u32,
        column: u32,
    },
    /// Apply a unary operator to `source` and write `dst`.
    Unary {
        dst: Register,
        op: UnaryOp,
        source: Register,
    },
    /// Apply a binary operator and write `dst`.
    Binary {
        dst: Register,
        left: Register,
        op: BinaryOp,
        right: Register,
    },
    /// Invoke a deterministic built-in using a contiguous register range.
    Call {
        dst: Register,
        function: Builtin,
        args: Range<Register>,
    },
    /// Draw an integer using explicit lower and upper bound registers.
    Random {
        dst: Register,
        args: Range<Register>,
        state_slot: u32,
    },
    /// Draw a boolean using an explicit percentage register.
    Chance {
        dst: Register,
        args: Range<Register>,
        state_slot: u32,
    },
    /// Jump when `condition` is boolean false. Other values fall through so
    /// the following binary operation preserves the original error order.
    JumpIfFalse { condition: Register, target: u32 },
    /// Jump when `condition` is boolean true. Other values fall through.
    JumpIfTrue { condition: Register, target: u32 },
    /// Render and concatenate a contiguous register range into `dst`.
    Concat {
        dst: Register,
        values: Range<Register>,
    },
}

/// A compiled expression with an explicit register file and result register.
///
/// `line` is the 1-based source line used for runtime diagnostics.
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExprChunk {
    pub ops: Vec<ExprOp>,
    pub constants: Vec<Value>,
    pub registers: Register,
    pub result: Register,
    pub line: u32,
}

/// Borrowed register bytecode from a standalone chunk or packed program arena.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExprChunkRef<'a> {
    pub ops: &'a [ExprOp],
    pub constants: &'a [Value],
    pub registers: Register,
    pub result: Register,
    pub line: u32,
}

impl ExprChunk {
    #[must_use]
    pub fn new(line: usize) -> Self {
        Self {
            ops: Vec::new(),
            constants: Vec::new(),
            registers: 1,
            result: 0,
            line: crate::compact_source_position(line),
        }
    }

    /// Resets a reusable chunk while retaining its arena allocations.
    pub fn reset(&mut self, line: usize) {
        self.ops.clear();
        self.constants.clear();
        self.registers = 1;
        self.result = 0;
        self.line = crate::compact_source_position(line);
    }

    /// Allocates one register after the fixed result register.
    ///
    /// # Panics
    /// Panics if the chunk exceeds the `u16` register namespace. Validation
    /// limits make this unreachable for accepted programs.
    pub fn register(&mut self) -> Register {
        let register = self.registers;
        self.registers = self
            .registers
            .checked_add(1)
            .expect("expression register count fits in u16");
        register
    }

    /// Allocates `count` consecutive registers.
    ///
    /// # Panics
    /// Panics if `count` or the resulting register count exceeds `u16`.
    pub fn register_range(&mut self, count: usize) -> Range<Register> {
        let count = Register::try_from(count).expect("argument count fits in u16");
        let start = self.registers;
        self.registers = self
            .registers
            .checked_add(count)
            .expect("expression register count fits in u16");
        start..self.registers
    }

    /// Interns a constant and returns its deduplicated pool index.
    ///
    /// # Panics
    /// Panics only if more than `u32::MAX` constants are interned.
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

    /// Appends an instruction and returns its index for jump patching.
    pub fn push(&mut self, op: ExprOp) -> usize {
        self.ops.push(op);
        self.ops.len() - 1
    }

    /// Borrows this standalone chunk in packed-program form.
    #[must_use]
    pub fn as_chunk_ref(&self) -> ExprChunkRef<'_> {
        ExprChunkRef {
            ops: &self.ops,
            constants: &self.constants,
            registers: self.registers,
            result: self.result,
            line: self.line,
        }
    }
}
