//! Validation for serialized or manually assembled register bytecode.

use crate::{ExecutionMetadata, ExprChunk, ExprChunkRef, ExprOp, Op, Program, UpdateOp};
use std::fmt;
use std::ops::Range;
use std::sync::{Arc, OnceLock};
use velin_syntax::{BinaryOp, Builtin, DataFootprint};

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
/// Maximum number of physical registers in one expression chunk.
pub const MAX_EXPR_REGISTERS: usize = 1_024;
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
    /// Verifies register indices, definitions, jumps and data budgets.
    ///
    /// `slot_count` is the width of the frame supplied during evaluation.
    ///
    /// # Errors
    /// Returns an error when the chunk is malformed, refers to unavailable
    /// data, reads an undefined register, or exceeds a resource limit.
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
    states: &mut Vec<Option<Vec<bool>>>,
) -> Result<(usize, usize), ProgramValidationError> {
    check_limit(
        &format!("ops in expression chunk {id}"),
        chunk.ops.len(),
        MAX_EXPR_OPS,
    )?;
    let register_count = usize::from(chunk.registers);
    if register_count == 0 {
        return Err(ProgramValidationError::new(format!(
            "expression chunk {id} has no registers"
        )));
    }
    check_limit(
        &format!("registers in expression chunk {id}"),
        register_count,
        MAX_EXPR_REGISTERS,
    )?;
    validate_register(chunk.result, register_count, id, "result")?;

    for (pc, op) in chunk.ops.iter().enumerate() {
        validate_instruction(op, chunk, slot_count, id, pc)?;
    }
    validate_register_flow(chunk, id, states)?;

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
    Ok((constant_values, text_bytes))
}

fn validate_instruction(
    op: &ExprOp,
    chunk: ExprChunkRef<'_>,
    slot_count: usize,
    id: usize,
    pc: usize,
) -> Result<(), ProgramValidationError> {
    let registers = usize::from(chunk.registers);
    let context = ValidationContext::ExpressionOp { chunk: id, pc };
    match op {
        ExprOp::Const { dst, constant } => {
            validate_register(*dst, registers, id, "destination")?;
            if *constant as usize >= chunk.constants.len() {
                return Err(ProgramValidationError::new(format!(
                    "expression chunk {id} op {pc} references missing constant {constant}"
                )));
            }
        }
        ExprOp::Load { dst, slot, .. } => {
            validate_register(*dst, registers, id, "destination")?;
            validate_slot(*slot, slot_count, context)?;
        }
        ExprOp::Unary { dst, source, .. } => {
            validate_register(*dst, registers, id, "destination")?;
            validate_register(*source, registers, id, "source")?;
        }
        ExprOp::Binary {
            dst, left, right, ..
        } => {
            validate_register(*dst, registers, id, "destination")?;
            validate_register(*left, registers, id, "left source")?;
            validate_register(*right, registers, id, "right source")?;
        }
        ExprOp::Call {
            dst,
            function,
            args,
        } => {
            if matches!(function, Builtin::Random | Builtin::Chance) {
                return Err(ProgramValidationError::new(format!(
                    "expression chunk {id} op {pc} must use the dedicated random opcode"
                )));
            }
            validate_register(*dst, registers, id, "destination")?;
            validate_range(args, registers, id, pc)?;
            if !function.accepts(args.len()) {
                return Err(ProgramValidationError::new(format!(
                    "expression chunk {id} op {pc} has invalid builtin argument count {}",
                    args.len()
                )));
            }
        }
        ExprOp::Random {
            dst,
            args,
            state_slot,
        } => {
            validate_register(*dst, registers, id, "destination")?;
            validate_range(args, registers, id, pc)?;
            validate_slot(*state_slot, slot_count, context)?;
            validate_random_arity(Builtin::Random, args, id, pc)?;
        }
        ExprOp::Chance {
            dst,
            args,
            state_slot,
        } => {
            validate_register(*dst, registers, id, "destination")?;
            validate_range(args, registers, id, pc)?;
            validate_slot(*state_slot, slot_count, context)?;
            validate_random_arity(Builtin::Chance, args, id, pc)?;
        }
        ExprOp::JumpIfFalse { condition, target } | ExprOp::JumpIfTrue { condition, target } => {
            validate_register(*condition, registers, id, "condition")?;
            let target = *target as usize;
            if target <= pc || target > chunk.ops.len() {
                return Err(ProgramValidationError::new(format!(
                    "expression chunk {id} op {pc} has invalid non-forward jump target {target}"
                )));
            }
        }
        ExprOp::Concat { dst, values } => {
            validate_register(*dst, registers, id, "destination")?;
            validate_range(values, registers, id, pc)?;
        }
    }
    Ok(())
}

fn validate_random_arity(
    function: Builtin,
    args: &Range<u16>,
    id: usize,
    pc: usize,
) -> Result<(), ProgramValidationError> {
    if function.accepts(args.len()) {
        return Ok(());
    }
    Err(ProgramValidationError::new(format!(
        "expression chunk {id} op {pc} has invalid random argument count {}",
        args.len()
    )))
}

fn validate_register_flow(
    chunk: ExprChunkRef<'_>,
    id: usize,
    states: &mut Vec<Option<Vec<bool>>>,
) -> Result<(), ProgramValidationError> {
    states.clear();
    states.resize_with(chunk.ops.len() + 1, || None);
    states[0] = Some(vec![false; usize::from(chunk.registers)]);

    for (pc, op) in chunk.ops.iter().enumerate() {
        let Some(state) = states[pc].take() else {
            continue;
        };
        for source in instruction_sources(op) {
            if !state[usize::from(source)] {
                return Err(ProgramValidationError::new(format!(
                    "expression chunk {id} op {pc} reads undefined register {source}"
                )));
            }
        }
        let mut outgoing = state;
        if let Some(dst) = instruction_destination(op) {
            outgoing[usize::from(dst)] = true;
        }
        match op {
            ExprOp::JumpIfFalse { target, .. } | ExprOp::JumpIfTrue { target, .. } => {
                merge_register_state(&mut states[*target as usize], &outgoing);
                merge_register_state(&mut states[pc + 1], &outgoing);
            }
            _ => merge_register_state(&mut states[pc + 1], &outgoing),
        }
    }

    let Some(final_state) = &states[chunk.ops.len()] else {
        return Err(ProgramValidationError::new(format!(
            "expression chunk {id} cannot reach its result"
        )));
    };
    if !final_state[usize::from(chunk.result)] {
        return Err(ProgramValidationError::new(format!(
            "expression chunk {id} leaves result register {} undefined",
            chunk.result
        )));
    }
    Ok(())
}

fn instruction_sources(op: &ExprOp) -> Vec<u16> {
    match op {
        ExprOp::Const { .. } | ExprOp::Load { .. } => Vec::new(),
        ExprOp::Unary { source, .. } => vec![*source],
        ExprOp::Binary { left, right, .. } => vec![*left, *right],
        ExprOp::Call { args, .. } | ExprOp::Random { args, .. } | ExprOp::Chance { args, .. } => {
            args.clone().collect()
        }
        ExprOp::JumpIfFalse { condition, .. } | ExprOp::JumpIfTrue { condition, .. } => {
            vec![*condition]
        }
        ExprOp::Concat { values, .. } => values.clone().collect(),
    }
}

fn instruction_destination(op: &ExprOp) -> Option<u16> {
    match op {
        ExprOp::Const { dst, .. }
        | ExprOp::Load { dst, .. }
        | ExprOp::Unary { dst, .. }
        | ExprOp::Binary { dst, .. }
        | ExprOp::Call { dst, .. }
        | ExprOp::Random { dst, .. }
        | ExprOp::Chance { dst, .. }
        | ExprOp::Concat { dst, .. } => Some(*dst),
        ExprOp::JumpIfFalse { .. } | ExprOp::JumpIfTrue { .. } => None,
    }
}

fn merge_register_state(slot: &mut Option<Vec<bool>>, incoming: &[bool]) {
    if let Some(current) = slot {
        for (defined, incoming) in current.iter_mut().zip(incoming) {
            *defined &= *incoming;
        }
    } else {
        *slot = Some(incoming.to_vec());
    }
}

fn validate_range(
    range: &Range<u16>,
    registers: usize,
    id: usize,
    pc: usize,
) -> Result<(), ProgramValidationError> {
    if range.start > range.end || usize::from(range.end) > registers {
        return Err(ProgramValidationError::new(format!(
            "expression chunk {id} op {pc} has invalid register range {}..{}",
            range.start, range.end
        )));
    }
    Ok(())
}

fn validate_register(
    register: u16,
    register_count: usize,
    id: usize,
    role: &str,
) -> Result<(), ProgramValidationError> {
    if usize::from(register) >= register_count {
        return Err(ProgramValidationError::new(format!(
            "expression chunk {id} references missing {role} register {register}"
        )));
    }
    Ok(())
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
