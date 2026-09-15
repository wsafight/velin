//! Portable Velin bytecode and its validation contract.
//!
//! This crate owns the data model shared by the compiler, checker, artifact
//! loader, and virtual machine. It does not parse source text or execute an
//! instruction. `velin-vm` consumes the validated [`Program`] values defined
//! here.

mod bytecode;
mod debug;
mod frame;
mod ir;
mod program;
mod slots;
mod validate;

fn compact_source_position(position: usize) -> u32 {
    u32::try_from(position).unwrap_or(u32::MAX)
}

pub use bytecode::{ExprChunk, ExprChunkRef, ExprOp, Register};
pub use debug::{DebugLocation, DebugTable};
pub use frame::{InitialFrame, InitialFrameError, InitialValue};
pub use ir::{IrBlock, IrLoop, IrOp, IrOptimization, IrTerminator, IrType, IrValue, TypedIr};
pub use program::{
    ChunkExecutionMetadata, ChunkId, ExecutionImage, ExecutionMetadata, HostOp, Op,
    OpExecutionMetadata, Pc, Program, ProgramArena, ProgramChunk, UpdateOp,
};
pub use slots::{RNG_STATE_SLOT, SlotTable};
pub use validate::{
    MAX_EXPR_OPS, MAX_EXPR_REGISTERS, MAX_HOST_ARGUMENTS, MAX_PROGRAM_CHUNKS,
    MAX_PROGRAM_CONSTANT_VALUES, MAX_PROGRAM_OPS, MAX_PROGRAM_SLOTS, MAX_PROGRAM_TEXT_BYTES,
    ProgramValidationError, ValidatedProgram,
};
