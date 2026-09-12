//! Validation for serialized or manually assembled bytecode programs.

use crate::{ExecutionMetadata, ExprChunk, ExprChunkRef, ExprOp, Op, Program, UpdateOp};
use std::fmt;
use std::sync::{Arc, OnceLock};
use velin_syntax::{BinaryOp, DataFootprint};

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

/// An immutable shared program that has passed full structural and budget
/// validation.
#[derive(Debug, Clone)]
pub struct ValidatedProgram {
    program: Arc<Program>,
    metadata: Arc<OnceLock<Arc<ExecutionMetadata>>>,
}

impl ValidatedProgram {
    /// Validates `program` and retains an immutable shared reference.
    ///
    /// # Errors
    /// Returns the same errors as [`Program::validate`].
    pub fn new(program: impl Into<Arc<Program>>) -> Result<Self, ProgramValidationError> {
        let program = program.into();
        program.validate()?;
        let metadata = Arc::new(OnceLock::new());
        Ok(Self { program, metadata })
    }

    /// Returns the validated program.
    #[must_use]
    pub fn program(&self) -> &Program {
        &self.program
    }

    /// Clones the immutable program reference while this validation proof
    /// remains alive.
    #[must_use]
    pub fn shared(&self) -> Arc<Program> {
        self.program.clone()
    }

    /// Clones the execution metadata computed by validation.
    #[must_use]
    pub fn shared_execution_metadata(&self) -> Arc<ExecutionMetadata> {
        self.metadata
            .get_or_init(|| Arc::new(ExecutionMetadata::new(&self.program)))
            .clone()
    }

    /// Returns whether `program` is the allocation covered by this proof.
    #[must_use]
    pub fn refers_to(&self, program: &Arc<Program>) -> bool {
        Arc::ptr_eq(&self.program, program)
    }
}

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
        let mut expression_heights = Vec::new();
        let mut text_bytes = self
            .slots
            .names()
            .iter()
            .try_fold(0usize, |total, name| total.checked_add(name.len()))
            .ok_or_else(|| ProgramValidationError::new("program text budget overflow"))?;

        for id in 0..self.chunks.len() {
            let chunk_id = u32::try_from(id)
                .map_err(|_| ProgramValidationError::new("expression chunk index overflow"))?;
            let chunk = self.chunk(chunk_id).ok_or_else(|| {
                ProgramValidationError::new(format!(
                    "expression chunk {id} has an invalid arena range"
                ))
            })?;
            let (values, bytes) =
                validate_chunk(chunk, self.slots.len(), id, &mut expression_heights)?;
            constant_values = constant_values
                .checked_add(values)
                .ok_or_else(|| ProgramValidationError::new("constant value budget overflow"))?;
            text_bytes = text_bytes
                .checked_add(bytes)
                .ok_or_else(|| ProgramValidationError::new("program text budget overflow"))?;
        }
        for (pc, op) in self.ops.iter().enumerate() {
            let footprint = validate_program_op(self, op, pc)?;
            constant_values = constant_values
                .checked_add(footprint.values)
                .ok_or_else(|| ProgramValidationError::new("constant value budget overflow"))?;
            text_bytes = text_bytes
                .checked_add(footprint.text_bytes)
                .ok_or_else(|| ProgramValidationError::new("program text budget overflow"))?;
        }
        check_limit(
            "constant values",
            constant_values,
            MAX_PROGRAM_CONSTANT_VALUES,
        )?;
        check_limit("program text bytes", text_bytes, MAX_PROGRAM_TEXT_BYTES)?;
        Ok(())
    }
}

fn validate_program_op(
    program: &Program,
    op: &Op,
    pc: usize,
) -> Result<DataFootprint, ProgramValidationError> {
    let context = ValidationContext::ProgramOp(pc);
    match op {
        Op::Set { slot, value } => {
            validate_slot(*slot, program.slots.len(), context)?;
            validate_chunk_id(*value, program.chunks.len(), context)?;
        }
        Op::SetConst { slot, value, .. } => {
            validate_slot(*slot, program.slots.len(), context)?;
            return value.data_footprint().map_err(|message| {
                ProgramValidationError::new(format!("op {pc} has an invalid constant: {message}"))
            });
        }
        Op::CopySlot { slot, source, .. } => {
            validate_slot(*slot, program.slots.len(), context)?;
            validate_slot(*source, program.slots.len(), context)?;
        }
        Op::Update {
            slot, operation, ..
        } => {
            validate_slot(*slot, program.slots.len(), context)?;
            for chunk in update_chunks(*operation).into_iter().flatten() {
                validate_chunk_id(chunk, program.chunks.len(), context)?;
            }
        }
        Op::Jump(target) => validate_program_target(*target, program.ops.len(), pc)?,
        Op::JumpIfFalse { condition, target } => {
            validate_chunk_id(*condition, program.chunks.len(), context)?;
            validate_program_target(*target, program.ops.len(), pc)?;
        }
        Op::JumpIfIntegerCompare {
            condition,
            slot,
            comparison,
            target,
            ..
        } => {
            validate_chunk_id(*condition, program.chunks.len(), context)?;
            validate_slot(*slot, program.slots.len(), context)?;
            validate_integer_comparison(*comparison, pc)?;
            validate_program_target(*target, program.ops.len(), pc)?;
        }
        Op::Host(host) => {
            if host.args.len() > MAX_HOST_ARGUMENTS {
                return Err(ProgramValidationError::new(format!(
                    "host arguments at op {pc} exceeds limit {MAX_HOST_ARGUMENTS} (found {})",
                    host.args.len()
                )));
            }
            for chunk in &host.args {
                validate_chunk_id(*chunk, program.chunks.len(), context)?;
            }
            if let Some(slot) = host.bind {
                validate_slot(slot, program.slots.len(), context)?;
            }
        }
        Op::Halt => {}
    }
    Ok(DataFootprint::default())
}

fn update_chunks(operation: UpdateOp) -> [Option<u32>; 2] {
    match operation {
        UpdateOp::Add { rhs } => [Some(rhs), None],
        UpdateOp::AddInteger { .. } => [None, None],
        UpdateOp::Push { value } => [Some(value), None],
        UpdateOp::Put { key, value } => [Some(key), Some(value)],
        UpdateOp::Remove { key } => [Some(key), None],
    }
}

fn validate_integer_comparison(
    comparison: BinaryOp,
    pc: usize,
) -> Result<(), ProgramValidationError> {
    if matches!(
        comparison,
        BinaryOp::Equal
            | BinaryOp::NotEqual
            | BinaryOp::Less
            | BinaryOp::LessEqual
            | BinaryOp::Greater
            | BinaryOp::GreaterEqual
    ) {
        return Ok(());
    }
    Err(ProgramValidationError::new(format!(
        "op {pc} has an invalid integer comparison"
    )))
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
mod tests {
    use super::*;
    use crate::SlotTable;
    use velin_syntax::Value;

    fn program(chunk: ExprChunk) -> Program {
        Program::from_chunks(
            vec![Op::Set { slot: 0, value: 0 }, Op::Halt],
            vec![chunk],
            {
                let mut slots = SlotTable::new();
                slots.intern("x");
                slots
            },
        )
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

        let inconsistent = ExprChunk {
            ops: vec![ExprOp::Const(0), ExprOp::JumpIfTrue(3), ExprOp::Const(0)],
            constants: vec![Value::Boolean(true)],
            line: 1,
        };
        assert!(
            program(inconsistent)
                .validate()
                .unwrap_err()
                .message
                .contains("inconsistent stack heights")
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

        let mut invalid_range = program(ExprChunk {
            ops: vec![ExprOp::Const(0)],
            constants: vec![Value::Integer(1)],
            line: 1,
        });
        invalid_range.chunks[0].ops.end += 1;
        assert!(
            invalid_range
                .validate()
                .unwrap_err()
                .message
                .contains("invalid arena range")
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
            constants: vec![Value::String("x".repeat(MAX_PROGRAM_TEXT_BYTES + 1).into())],
            line: 1,
        };
        assert!(oversized.validate(0).is_err());
    }

    #[test]
    fn validated_program_requires_a_valid_program_and_keeps_it_shared() {
        let valid = Arc::new(program(ExprChunk {
            ops: vec![ExprOp::Const(0)],
            constants: vec![Value::Integer(1)],
            line: 1,
        }));
        let validated = ValidatedProgram::new(valid.clone()).unwrap();
        assert!(Arc::ptr_eq(&valid, &validated.shared()));

        let invalid = Program {
            ops: vec![Op::Jump(2)],
            chunks: Vec::new(),
            expr_ops: Vec::new(),
            constants: Vec::new(),
            slots: SlotTable::new(),
        };
        assert!(ValidatedProgram::new(invalid).is_err());
    }
}
