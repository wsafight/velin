//! Validation for serialized or manually assembled bytecode programs.

use crate::{ExecutionMetadata, ExprChunk, ExprChunkRef, ExprOp, Op, Program, UpdateOp};
use std::fmt;
use std::sync::{Arc, OnceLock};
use velin_syntax::{BinaryOp, DataFootprint};

mod program_validation;
mod proof;

pub use proof::ValidatedProgram;

/// Maximum number of control-flow instructions in an executable program.
pub const MAX_PROGRAM_OPS: usize = 100_000;
/// Maximum number of expression chunks in an executable program.
pub const MAX_PROGRAM_CHUNKS: usize = 100_000;
/// Maximum number of variable slots in an executable program.
pub const MAX_PROGRAM_SLOTS: usize = 65_536;
/// Maximum number of instructions in one expression chunk.
pub const MAX_EXPR_OPS: usize = 4_096;
/// Maximum operand-stack height reached by one expression chunk.
pub const MAX_EXPR_STACK: usize = 1_024;
/// Maximum aggregate number of values stored in all constant pools.
pub const MAX_PROGRAM_CONSTANT_VALUES: usize = 100_000;
/// Maximum aggregate UTF-8 bytes stored in constant pools and slot names.
pub const MAX_PROGRAM_TEXT_BYTES: usize = 16 * 1024 * 1024;
/// Maximum argument count accepted by a host instruction.
pub const MAX_HOST_ARGUMENTS: usize = 128;

/// A structural or resource-budget error in executable bytecode.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProgramValidationError {
    pub message: String,
}

impl ProgramValidationError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl std::fmt::Display for ProgramValidationError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for ProgramValidationError {}

impl ExprChunk {
    /// Verifies this chunk's indices, stack paths, jumps and data budgets.
    ///
    /// `slot_count` is the width of the frame that will be supplied during
    /// evaluation.
    ///
    /// # Errors
    /// Returns a descriptive error when the chunk is malformed, refers to a
    /// missing frame slot, or exceeds a bytecode/data budget.
    pub fn validate(&self, slot_count: usize) -> Result<(), ProgramValidationError> {
        check_limit("variable slots", slot_count, MAX_PROGRAM_SLOTS)?;
        let (values, bytes) = validate_chunk(self.as_chunk_ref(), slot_count, 0, &mut Vec::new())?;
        check_limit("constant values", values, MAX_PROGRAM_CONSTANT_VALUES)?;
        check_limit("expression text bytes", bytes, MAX_PROGRAM_TEXT_BYTES)
    }
}

fn validate_chunk(
    chunk: ExprChunkRef<'_>,
    slot_count: usize,
    id: usize,
    heights: &mut Vec<Option<usize>>,
) -> Result<(usize, usize), ProgramValidationError> {
    if chunk.ops.len() > MAX_EXPR_OPS {
        return Err(ProgramValidationError::new(format!(
            "ops in expression chunk {id} exceeds limit {MAX_EXPR_OPS} (found {})",
            chunk.ops.len()
        )));
    }

    let mut has_branches = false;
    let mut linear_height = 0;
    let mut linear_error = None;
    for (pc, op) in chunk.ops.iter().enumerate() {
        match op {
            ExprOp::Const(index) if *index as usize >= chunk.constants.len() => {
                return Err(ProgramValidationError::new(format!(
                    "expression chunk {id} op {pc} references missing constant {index}"
                )));
            }
            ExprOp::Load { slot, .. }
            | ExprOp::Random { state_slot: slot }
            | ExprOp::Chance { state_slot: slot } => {
                validate_slot(
                    *slot,
                    slot_count,
                    ValidationContext::ExpressionOp { chunk: id, pc },
                )?;
            }
            ExprOp::Call { function, argc } if !function.accepts(*argc as usize) => {
                return Err(ProgramValidationError::new(format!(
                    "expression chunk {id} op {pc} has invalid builtin argument count {argc}"
                )));
            }
            ExprOp::JumpIfFalse(target) | ExprOp::JumpIfTrue(target) => {
                has_branches = true;
                let target = *target as usize;
                if target <= pc || target > chunk.ops.len() {
                    return Err(ProgramValidationError::new(format!(
                        "expression chunk {id} op {pc} has invalid non-forward jump target {target}"
                    )));
                }
            }
            ExprOp::AssertBoolean(op) if !matches!(op, BinaryOp::And | BinaryOp::Or) => {
                return Err(ProgramValidationError::new(format!(
                    "expression chunk {id} op {pc} has invalid boolean assertion"
                )));
            }
            _ => {}
        }
        if !has_branches {
            advance_linear_stack(&mut linear_height, &mut linear_error, op, pc);
        }
    }

    let mut constant_values = 0usize;
    let mut text_bytes = 0usize;
    for value in chunk.constants {
        let footprint = value.data_footprint().map_err(|message| {
            ProgramValidationError::new(format!(
                "expression chunk {id} has an invalid constant: {message}"
            ))
        })?;
        constant_values = constant_values
            .checked_add(footprint.values)
            .ok_or_else(|| ProgramValidationError::new("constant value budget overflow"))?;
        text_bytes = text_bytes
            .checked_add(footprint.text_bytes)
            .ok_or_else(|| ProgramValidationError::new("program text budget overflow"))?;
    }

    if !has_branches {
        finish_linear_stack(linear_height, linear_error, id)?;
        return Ok((constant_values, text_bytes));
    }

    validate_branched_stack(chunk, id, heights)?;
    Ok((constant_values, text_bytes))
}

fn validate_branched_stack(
    chunk: ExprChunkRef<'_>,
    id: usize,
    heights: &mut Vec<Option<usize>>,
) -> Result<(), ProgramValidationError> {
    heights.clear();
    heights.resize(chunk.ops.len() + 1, None);
    heights[0] = Some(0);
    for (pc, op) in chunk.ops.iter().enumerate() {
        let Some(height) = heights[pc] else {
            continue;
        };
        if height > MAX_EXPR_STACK {
            return Err(ProgramValidationError::new(format!(
                "expression chunk {id} exceeds operand stack limit {MAX_EXPR_STACK}"
            )));
        }
        let required = required_operands(op);
        if height < required {
            return Err(ProgramValidationError::new(format!(
                "expression chunk {id} op {pc} needs {required} stack values but has {height}"
            )));
        }
        match op {
            ExprOp::JumpIfFalse(target) | ExprOp::JumpIfTrue(target) => {
                set_validated_height(&mut heights[*target as usize], height, id, *target as usize)?;
                set_validated_height(&mut heights[pc + 1], height, id, pc + 1)?;
            }
            _ => set_validated_height(&mut heights[pc + 1], next_height(op, height), id, pc + 1)?,
        }
    }
    let height = heights[chunk.ops.len()].unwrap_or(0);
    if height > MAX_EXPR_STACK {
        return Err(ProgramValidationError::new(format!(
            "expression chunk {id} exceeds operand stack limit {MAX_EXPR_STACK}"
        )));
    }
    if height != 1 {
        return Err(ProgramValidationError::new(format!(
            "expression chunk {id} finishes with {height} values instead of one"
        )));
    }
    Ok(())
}

fn set_validated_height(
    slot: &mut Option<usize>,
    height: usize,
    id: usize,
    pc: usize,
) -> Result<(), ProgramValidationError> {
    if slot.is_some_and(|previous| previous != height) {
        return Err(ProgramValidationError::new(format!(
            "expression chunk {id} reaches op {pc} with inconsistent stack heights"
        )));
    }
    *slot = Some(height);
    Ok(())
}

#[derive(Clone, Copy)]
struct LinearStackError {
    pc: usize,
    required: usize,
    available: usize,
}

fn advance_linear_stack(
    height: &mut usize,
    error: &mut Option<LinearStackError>,
    op: &ExprOp,
    pc: usize,
) {
    if error.is_some() || *height > MAX_EXPR_STACK {
        return;
    }
    let required = required_operands(op);
    if *height < required {
        *error = Some(LinearStackError {
            pc,
            required,
            available: *height,
        });
    } else {
        *height = next_height(op, *height);
    }
}

fn finish_linear_stack(
    height: usize,
    error: Option<LinearStackError>,
    id: usize,
) -> Result<(), ProgramValidationError> {
    if let Some(error) = error {
        return Err(ProgramValidationError::new(format!(
            "expression chunk {id} op {} needs {} stack values but has {}",
            error.pc, error.required, error.available
        )));
    }
    if height > MAX_EXPR_STACK {
        return Err(ProgramValidationError::new(format!(
            "expression chunk {id} exceeds operand stack limit {MAX_EXPR_STACK}"
        )));
    }
    if height != 1 {
        return Err(ProgramValidationError::new(format!(
            "expression chunk {id} finishes with {height} values instead of one"
        )));
    }
    Ok(())
}

fn required_operands(op: &ExprOp) -> usize {
    match op {
        ExprOp::Const(_) | ExprOp::Load { .. } => 0,
        ExprOp::Unary(_)
        | ExprOp::Chance { .. }
        | ExprOp::AssertBoolean(_)
        | ExprOp::JumpIfFalse(_)
        | ExprOp::JumpIfTrue(_) => 1,
        ExprOp::Binary(_) | ExprOp::Random { .. } => 2,
        ExprOp::Call { argc, .. } | ExprOp::Concat(argc) => *argc as usize,
    }
}

fn next_height(op: &ExprOp, height: usize) -> usize {
    match op {
        ExprOp::Const(_) | ExprOp::Load { .. } => height + 1,
        ExprOp::Unary(_) | ExprOp::Chance { .. } | ExprOp::AssertBoolean(_) => height,
        ExprOp::Binary(_) | ExprOp::Random { .. } => height - 1,
        ExprOp::Call { argc, .. } | ExprOp::Concat(argc) => height - *argc as usize + 1,
        ExprOp::JumpIfFalse(_) | ExprOp::JumpIfTrue(_) => unreachable!("handled as branches"),
    }
}

fn validate_program_target(
    target: u32,
    op_count: usize,
    pc: usize,
) -> Result<(), ProgramValidationError> {
    if target as usize > op_count {
        return Err(ProgramValidationError::new(format!(
            "op {pc} references jump target {target} past program end {op_count}"
        )));
    }
    Ok(())
}

fn validate_chunk_id(
    chunk: u32,
    chunk_count: usize,
    context: ValidationContext,
) -> Result<(), ProgramValidationError> {
    if chunk as usize >= chunk_count {
        return Err(ProgramValidationError::new(format!(
            "{context} references missing expression chunk {chunk}"
        )));
    }
    Ok(())
}

fn validate_slot(
    slot: u32,
    slot_count: usize,
    context: ValidationContext,
) -> Result<(), ProgramValidationError> {
    if slot as usize >= slot_count {
        return Err(ProgramValidationError::new(format!(
            "{context} references missing slot {slot}"
        )));
    }
    Ok(())
}

#[derive(Clone, Copy)]
enum ValidationContext {
    ProgramOp(usize),
    ExpressionOp { chunk: usize, pc: usize },
}

impl fmt::Display for ValidationContext {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ProgramOp(pc) => write!(formatter, "op {pc}"),
            Self::ExpressionOp { chunk, pc } => {
                write!(formatter, "expression chunk {chunk} op {pc}")
            }
        }
    }
}

fn check_limit(subject: &str, actual: usize, limit: usize) -> Result<(), ProgramValidationError> {
    if actual > limit {
        return Err(ProgramValidationError::new(format!(
            "{subject} exceeds limit {limit} (found {actual})"
        )));
    }
    Ok(())
}

#[cfg(test)]
#[path = "validate_tests.rs"]
mod tests;
