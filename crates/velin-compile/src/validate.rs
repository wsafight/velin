//! Validation for serialized or manually assembled bytecode programs.

use crate::{ExprChunk, ExprOp, Op, Program};
use std::collections::VecDeque;
use velin_syntax::{BinaryOp, Value};

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

impl Program {
    /// Verifies every index, expression stack path, jump and bytecode budget.
    ///
    /// Compiler output is valid by construction. Call this before executing a
    /// program obtained from serialization or a low-level bytecode producer.
    ///
    /// # Errors
    /// Returns a descriptive error when the program is malformed or exceeds a
    /// bytecode/data budget.
    pub fn validate(&self) -> Result<(), ProgramValidationError> {
        check_limit("program ops", self.ops.len(), MAX_PROGRAM_OPS)?;
        check_limit("expression chunks", self.chunks.len(), MAX_PROGRAM_CHUNKS)?;
        check_limit("variable slots", self.slots.len(), MAX_PROGRAM_SLOTS)?;

        let mut constant_values = 0usize;
        let mut text_bytes = self
            .slots
            .names()
            .iter()
            .try_fold(0usize, |total, name| total.checked_add(name.len()))
            .ok_or_else(|| ProgramValidationError::new("program text budget overflow"))?;

        for (id, chunk) in self.chunks.iter().enumerate() {
            let (values, bytes) = validate_chunk(chunk, self.slots.len(), id)?;
            constant_values = constant_values
                .checked_add(values)
                .ok_or_else(|| ProgramValidationError::new("constant value budget overflow"))?;
            text_bytes = text_bytes
                .checked_add(bytes)
                .ok_or_else(|| ProgramValidationError::new("program text budget overflow"))?;
        }
        check_limit(
            "constant values",
            constant_values,
            MAX_PROGRAM_CONSTANT_VALUES,
        )?;
        check_limit("program text bytes", text_bytes, MAX_PROGRAM_TEXT_BYTES)?;

        for (pc, op) in self.ops.iter().enumerate() {
            match op {
                Op::Set { slot, value } => {
                    validate_slot(*slot, self.slots.len(), &format!("op {pc}"))?;
                    validate_chunk_id(*value, self.chunks.len(), &format!("op {pc}"))?;
                }
                Op::Jump(target) => validate_program_target(*target, self.ops.len(), pc)?,
                Op::JumpIfFalse { condition, target } => {
                    validate_chunk_id(*condition, self.chunks.len(), &format!("op {pc}"))?;
                    validate_program_target(*target, self.ops.len(), pc)?;
                }
                Op::Host { args, bind, .. } => {
                    check_limit(
                        &format!("host arguments at op {pc}"),
                        args.len(),
                        MAX_HOST_ARGUMENTS,
                    )?;
                    for chunk in args {
                        validate_chunk_id(*chunk, self.chunks.len(), &format!("op {pc}"))?;
                    }
                    if let Some(slot) = bind {
                        validate_slot(*slot, self.slots.len(), &format!("op {pc}"))?;
                    }
                }
                Op::Halt => {}
            }
        }
        Ok(())
    }
}

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
        let (values, bytes) = validate_chunk(self, slot_count, 0)?;
        check_limit("constant values", values, MAX_PROGRAM_CONSTANT_VALUES)?;
        check_limit("expression text bytes", bytes, MAX_PROGRAM_TEXT_BYTES)
    }
}

fn validate_chunk(
    chunk: &ExprChunk,
    slot_count: usize,
    id: usize,
) -> Result<(usize, usize), ProgramValidationError> {
    check_limit(
        &format!("ops in expression chunk {id}"),
        chunk.ops.len(),
        MAX_EXPR_OPS,
    )?;

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
                validate_slot(*slot, slot_count, &format!("expression chunk {id} op {pc}"))?;
            }
            ExprOp::Call { function, argc } if !function.accepts(*argc as usize) => {
                return Err(ProgramValidationError::new(format!(
                    "expression chunk {id} op {pc} has invalid builtin argument count {argc}"
                )));
            }
            ExprOp::JumpIfFalse(target) | ExprOp::JumpIfTrue(target) => {
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
    }

    let mut constant_values = 0usize;
    let mut text_bytes = 0usize;
    for value in &chunk.constants {
        value.validate_data().map_err(|message| {
            ProgramValidationError::new(format!(
                "expression chunk {id} has an invalid constant: {message}"
            ))
        })?;
        let (values, bytes) = data_usage(value)?;
        constant_values = constant_values
            .checked_add(values)
            .ok_or_else(|| ProgramValidationError::new("constant value budget overflow"))?;
        text_bytes = text_bytes
            .checked_add(bytes)
            .ok_or_else(|| ProgramValidationError::new("program text budget overflow"))?;
    }

    let mut heights = vec![None; chunk.ops.len() + 1];
    let mut pending = VecDeque::from([(0usize, 0usize)]);
    while let Some((pc, height)) = pending.pop_front() {
        if let Some(previous) = heights[pc] {
            if previous != height {
                return Err(ProgramValidationError::new(format!(
                    "expression chunk {id} reaches op {pc} with inconsistent stack heights"
                )));
            }
            continue;
        }
        if height > MAX_EXPR_STACK {
            return Err(ProgramValidationError::new(format!(
                "expression chunk {id} exceeds operand stack limit {MAX_EXPR_STACK}"
            )));
        }
        heights[pc] = Some(height);
        if pc == chunk.ops.len() {
            if height != 1 {
                return Err(ProgramValidationError::new(format!(
                    "expression chunk {id} finishes with {height} values instead of one"
                )));
            }
            continue;
        }

        let op = &chunk.ops[pc];
        let required = required_operands(op);
        if height < required {
            return Err(ProgramValidationError::new(format!(
                "expression chunk {id} op {pc} needs {required} stack values but has {height}"
            )));
        }
        match op {
            ExprOp::JumpIfFalse(target) | ExprOp::JumpIfTrue(target) => {
                pending.push_back((*target as usize, height));
                pending.push_back((pc + 1, height));
            }
            _ => pending.push_back((pc + 1, next_height(op, height))),
        }
    }
    Ok((constant_values, text_bytes))
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
    context: &str,
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
    context: &str,
) -> Result<(), ProgramValidationError> {
    if slot as usize >= slot_count {
        return Err(ProgramValidationError::new(format!(
            "{context} references missing slot {slot}"
        )));
    }
    Ok(())
}

fn check_limit(subject: &str, actual: usize, limit: usize) -> Result<(), ProgramValidationError> {
    if actual > limit {
        return Err(ProgramValidationError::new(format!(
            "{subject} exceeds limit {limit} (found {actual})"
        )));
    }
    Ok(())
}

fn data_usage(value: &Value) -> Result<(usize, usize), ProgramValidationError> {
    let mut pending = vec![value];
    let mut values = 0usize;
    let mut bytes = 0usize;
    while let Some(value) = pending.pop() {
        values = values
            .checked_add(1)
            .ok_or_else(|| ProgramValidationError::new("constant value budget overflow"))?;
        match value {
            Value::String(text) => {
                bytes = bytes
                    .checked_add(text.len())
                    .ok_or_else(|| ProgramValidationError::new("program text budget overflow"))?;
            }
            Value::List(items) => pending.extend(items.iter()),
            Value::Record(fields) => {
                for (key, value) in fields.iter() {
                    bytes = bytes.checked_add(key.len()).ok_or_else(|| {
                        ProgramValidationError::new("program text budget overflow")
                    })?;
                    pending.push(value);
                }
            }
            Value::Integer(_) | Value::Boolean(_) => {}
        }
    }
    Ok((values, bytes))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SlotTable;

    fn program(chunk: ExprChunk) -> Program {
        Program {
            ops: vec![Op::Set { slot: 0, value: 0 }, Op::Halt],
            chunks: vec![chunk],
            slots: {
                let mut slots = SlotTable::new();
                slots.intern("x");
                slots
            },
        }
    }

    #[test]
    fn validates_stack_paths_and_rejects_back_edges() {
        let valid = ExprChunk {
            ops: vec![ExprOp::Const(0)],
            constants: vec![Value::Integer(1)],
            line: 1,
        };
        assert!(program(valid).validate().is_ok());

        let looping = ExprChunk {
            ops: vec![ExprOp::Const(0), ExprOp::JumpIfTrue(0)],
            constants: vec![Value::Boolean(true)],
            line: 1,
        };
        assert!(
            program(looping)
                .validate()
                .unwrap_err()
                .message
                .contains("non-forward")
        );

        let underflow = ExprChunk {
            ops: vec![ExprOp::Binary(BinaryOp::Add)],
            constants: Vec::new(),
            line: 1,
        };
        assert!(
            program(underflow)
                .validate()
                .unwrap_err()
                .message
                .contains("needs 2")
        );
    }

    #[test]
    fn rejects_invalid_program_indices() {
        let chunk = ExprChunk {
            ops: vec![ExprOp::Const(0)],
            constants: vec![Value::Integer(1)],
            line: 1,
        };
        let mut invalid = program(chunk);
        invalid.ops[0] = Op::Set { slot: 9, value: 0 };
        assert!(
            invalid
                .validate()
                .unwrap_err()
                .message
                .contains("missing slot")
        );
        invalid.ops[0] = Op::Jump(99);
        assert!(
            invalid
                .validate()
                .unwrap_err()
                .message
                .contains("past program end")
        );
    }

    #[test]
    fn standalone_chunk_validation_checks_frame_width_and_constants() {
        let missing_slot = ExprChunk {
            ops: vec![ExprOp::Load { slot: 1, column: 1 }],
            constants: Vec::new(),
            line: 1,
        };
        assert!(
            missing_slot
                .validate(1)
                .unwrap_err()
                .message
                .contains("missing slot")
        );

        let oversized = ExprChunk {
            ops: vec![ExprOp::Const(0)],
            constants: vec![Value::String("x".repeat(MAX_PROGRAM_TEXT_BYTES + 1))],
            line: 1,
        };
        assert!(oversized.validate(0).is_err());
    }
}
