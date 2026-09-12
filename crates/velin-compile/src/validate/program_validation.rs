use super::{
    BinaryOp, DataFootprint, MAX_HOST_ARGUMENTS, MAX_PROGRAM_CHUNKS, MAX_PROGRAM_CONSTANT_VALUES,
    MAX_PROGRAM_OPS, MAX_PROGRAM_SLOTS, MAX_PROGRAM_TEXT_BYTES, Op, Program,
    ProgramValidationError, UpdateOp, ValidationContext, check_limit, validate_chunk,
    validate_chunk_id, validate_program_target, validate_slot,
};

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
