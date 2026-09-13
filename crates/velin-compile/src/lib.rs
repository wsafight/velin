//! Bytecode compiler for Velin.
//!
//! This crate turns the `Expr` tree (and program-level control flow) into a
//! flat, slot-addressed intermediate representation that [`velin-vm`] executes.
//! It has no host, I/O, or foreign-code capability, and it introduces no
//! host-domain concepts: outside effects are represented only by the opaque
//! [`Op::Host`] opcode.
//!
//! The two IR layers are:
//!
//! * [`ExprChunk`] / [`ExprOp`] — a stack machine for one expression.
//! * [`Program`] / [`Op`] — control flow over expression chunks, assembled with
//!   [`ProgramBuilder`].
//!
//! Variable names are resolved to dense frame indices at compile time via
//! [`SlotTable`], so the VM never hashes strings at runtime.

mod bytecode;
mod expr;
mod frame;
mod program;
mod slots;
mod validate;

fn compact_source_position(position: usize) -> u32 {
    u32::try_from(position).unwrap_or(u32::MAX)
}

pub use bytecode::{ExprChunk, ExprChunkRef, ExprOp};
pub use expr::compile_expression;
pub use frame::{InitialFrame, InitialFrameError, InitialValue};
pub use program::{
    ChunkExecutionMetadata, ChunkId, ExecutionMetadata, HostOp, Op, OpExecutionMetadata, Pc,
    PreparedExpr, Program, ProgramBuilder, ProgramChunk, QuickenedCallRef, QuickenedOperand,
    UpdateOp,
};
pub use slots::{RNG_STATE_SLOT, SlotTable};
pub use validate::{
    MAX_EXPR_OPS, MAX_EXPR_STACK, MAX_HOST_ARGUMENTS, MAX_PROGRAM_CHUNKS,
    MAX_PROGRAM_CONSTANT_VALUES, MAX_PROGRAM_OPS, MAX_PROGRAM_SLOTS, MAX_PROGRAM_TEXT_BYTES,
    ProgramValidationError, ValidatedProgram,
};
