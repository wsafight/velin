use super::{Arc, ExecutionMetadata, OnceLock, Program, ProgramValidationError};

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
            .get_or_init(|| Arc::new(ExecutionMetadata::new(self.program.clone())))
            .clone()
    }

    /// Returns whether `program` is the allocation covered by this proof.
    #[must_use]
    pub fn refers_to(&self, program: &Arc<Program>) -> bool {
        Arc::ptr_eq(&self.program, program)
    }
}
