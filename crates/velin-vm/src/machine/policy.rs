use std::fmt;
use std::sync::Arc;

use super::{
    DEFAULT_RNG_SEED, MAX_HOST_PAYLOAD_TEXT_BYTES, MAX_HOST_PAYLOAD_VALUES, MAX_IMMEDIATE_STEPS,
    MAX_MACHINE_DATA_VALUES, MAX_MACHINE_TEXT_BYTES, Machine,
};
use velin_bytecode::{
    ExecutionImage, InitialFrame, Program, ProgramValidationError, ValidatedProgram,
};
use velin_syntax::{MAX_DATA_TEXT_BYTES, MAX_DATA_VALUES};

/// Default cumulative fuel available to one machine lifetime.
pub const DEFAULT_MAX_FUEL: u64 = 10_000_000;

/// Progress reported to an optional host callback.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExecutionProgress {
    /// Fuel consumed by this machine, including previous `run` / `resume` calls.
    pub fuel_used: u64,
    /// Fuel consumed by the current immediate execution call.
    pub immediate_fuel: u64,
    /// Host effects yielded by this machine so far.
    pub host_effects: usize,
    /// Current VM call depth. The current VM has one top-level frame.
    pub call_depth: usize,
}

/// A callback invoked periodically while a machine is executing.
///
/// Returning `false` requests cooperative cancellation. The callback is
/// intentionally owned by an `Arc` so a policy can be copied into a runner,
/// invoker, or snapshot without borrowing host state.
pub type ProgressCallback = Arc<dyn Fn(ExecutionProgress) -> bool + Send + Sync + 'static>;

/// Shared resource and cancellation policy for all VM entry points.
#[derive(Clone)]
pub struct ExecutionPolicy {
    /// Total fuel allowed across all `run` / `resume` calls until restart.
    pub max_fuel: u64,
    /// Fuel allowed in one immediate `run`, `resume`, or batch call.
    pub max_immediate_fuel: u64,
    /// Total host effects allowed until restart.
    pub max_host_effects: usize,
    /// Maximum nested VM call depth. The current VM uses depth one.
    pub max_call_depth: usize,
    /// Maximum footprint of one value installed or produced by the VM.
    pub max_value_values: usize,
    /// Maximum text footprint of one value installed or produced by the VM.
    pub max_value_text_bytes: usize,
    /// Maximum aggregate value footprint retained by one machine frame.
    pub max_machine_values: usize,
    /// Maximum aggregate text footprint retained by one machine frame.
    pub max_machine_text_bytes: usize,
    /// Maximum aggregate value footprint in one host payload.
    pub max_host_payload_values: usize,
    /// Maximum aggregate text footprint in one host payload.
    pub max_host_payload_text_bytes: usize,
    /// Maximum events retained by a policy-backed host queue.
    pub max_host_queue_events: usize,
    /// Maximum values retained by a policy-backed host queue.
    pub max_host_queue_values: usize,
    /// Maximum text bytes retained by a policy-backed host queue.
    pub max_host_queue_text_bytes: usize,
    /// Number of fuel units between progress callback invocations.
    pub progress_interval: u64,
    /// Optional callback for cooperative cancellation or progress reporting.
    pub progress_callback: Option<ProgressCallback>,
}

impl fmt::Debug for ExecutionPolicy {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ExecutionPolicy")
            .field("max_fuel", &self.max_fuel)
            .field("max_immediate_fuel", &self.max_immediate_fuel)
            .field("max_host_effects", &self.max_host_effects)
            .field("max_call_depth", &self.max_call_depth)
            .field("max_value_values", &self.max_value_values)
            .field("max_value_text_bytes", &self.max_value_text_bytes)
            .field("max_machine_values", &self.max_machine_values)
            .field("max_machine_text_bytes", &self.max_machine_text_bytes)
            .field("max_host_payload_values", &self.max_host_payload_values)
            .field(
                "max_host_payload_text_bytes",
                &self.max_host_payload_text_bytes,
            )
            .field("max_host_queue_events", &self.max_host_queue_events)
            .field("max_host_queue_values", &self.max_host_queue_values)
            .field("max_host_queue_text_bytes", &self.max_host_queue_text_bytes)
            .field("progress_interval", &self.progress_interval)
            .field(
                "progress_callback",
                &self.progress_callback.as_ref().map(|_| "configured"),
            )
            .finish()
    }
}

impl Default for ExecutionPolicy {
    fn default() -> Self {
        Self {
            max_fuel: DEFAULT_MAX_FUEL,
            max_immediate_fuel: MAX_IMMEDIATE_STEPS as u64,
            max_host_effects: 1_000,
            max_call_depth: 64,
            max_value_values: MAX_DATA_VALUES,
            max_value_text_bytes: MAX_DATA_TEXT_BYTES,
            max_machine_values: MAX_MACHINE_DATA_VALUES,
            max_machine_text_bytes: MAX_MACHINE_TEXT_BYTES,
            max_host_payload_values: MAX_HOST_PAYLOAD_VALUES,
            max_host_payload_text_bytes: MAX_HOST_PAYLOAD_TEXT_BYTES,
            max_host_queue_events: 1_024,
            max_host_queue_values: 1_000_000,
            max_host_queue_text_bytes: 64 * 1024 * 1024,
            progress_interval: 256,
            progress_callback: None,
        }
    }
}

impl ExecutionPolicy {
    /// Returns a policy with only the cumulative fuel budget disabled.
    #[must_use]
    pub const fn unlimited_fuel(mut self) -> Self {
        self.max_fuel = u64::MAX;
        self
    }

    /// Sets the cumulative fuel budget.
    #[must_use]
    pub const fn with_max_fuel(mut self, max_fuel: u64) -> Self {
        self.max_fuel = max_fuel;
        self
    }

    /// Sets the per-call immediate fuel budget.
    #[must_use]
    pub const fn with_max_immediate_fuel(mut self, max_immediate_fuel: u64) -> Self {
        self.max_immediate_fuel = max_immediate_fuel;
        self
    }

    /// Sets the cumulative host-effect budget.
    #[must_use]
    pub const fn with_max_host_effects(mut self, max_host_effects: usize) -> Self {
        self.max_host_effects = max_host_effects;
        self
    }

    /// Sets the maximum nested VM call depth.
    #[must_use]
    pub const fn with_max_call_depth(mut self, max_call_depth: usize) -> Self {
        self.max_call_depth = max_call_depth;
        self
    }

    /// Sets the per-value value-count and text-byte budgets.
    #[must_use]
    pub const fn with_value_budgets(
        mut self,
        max_value_values: usize,
        max_value_text_bytes: usize,
    ) -> Self {
        self.max_value_values = max_value_values;
        self.max_value_text_bytes = max_value_text_bytes;
        self
    }

    /// Sets the machine-state value and text budgets.
    #[must_use]
    pub const fn with_machine_budgets(
        mut self,
        max_machine_values: usize,
        max_machine_text_bytes: usize,
    ) -> Self {
        self.max_machine_values = max_machine_values;
        self.max_machine_text_bytes = max_machine_text_bytes;
        self
    }

    /// Sets the host-payload value and text budgets.
    #[must_use]
    pub const fn with_host_payload_budgets(
        mut self,
        max_host_payload_values: usize,
        max_host_payload_text_bytes: usize,
    ) -> Self {
        self.max_host_payload_values = max_host_payload_values;
        self.max_host_payload_text_bytes = max_host_payload_text_bytes;
        self
    }

    /// Sets the policy-backed host queue budgets.
    #[must_use]
    pub const fn with_host_queue_budgets(
        mut self,
        max_host_queue_events: usize,
        max_host_queue_values: usize,
        max_host_queue_text_bytes: usize,
    ) -> Self {
        self.max_host_queue_events = max_host_queue_events;
        self.max_host_queue_values = max_host_queue_values;
        self.max_host_queue_text_bytes = max_host_queue_text_bytes;
        self
    }

    /// Installs a host callback and its fuel polling interval.
    #[must_use]
    pub fn with_progress_callback<F>(mut self, interval: u64, callback: F) -> Self
    where
        F: Fn(ExecutionProgress) -> bool + Send + Sync + 'static,
    {
        self.progress_interval = interval.max(1);
        self.progress_callback = Some(Arc::new(callback));
        self
    }
}

impl Machine {
    /// Creates a machine for a validated `program` with an empty variable frame.
    ///
    /// # Errors
    /// Returns an error when `program` fails structural validation.
    pub fn new(program: impl Into<Arc<Program>>) -> Result<Self, ProgramValidationError> {
        Self::with_seed(program, DEFAULT_RNG_SEED)
    }

    /// Creates a machine with an explicit execution policy.
    ///
    /// # Errors
    /// Returns an error when `program` fails structural validation.
    pub fn new_with_policy(
        program: impl Into<Arc<Program>>,
        policy: ExecutionPolicy,
    ) -> Result<Self, ProgramValidationError> {
        Self::with_seed_and_policy(program, DEFAULT_RNG_SEED, policy)
    }

    /// Creates a machine without allocating an execution profile.
    ///
    /// # Errors
    /// Returns an error when `program` fails structural validation.
    pub fn new_without_profile(
        program: impl Into<Arc<Program>>,
    ) -> Result<Self, ProgramValidationError> {
        Self::with_seed_without_profile(program, DEFAULT_RNG_SEED)
    }

    /// Creates a machine without profiling and with an explicit policy.
    ///
    /// # Errors
    /// Returns an error when `program` fails structural validation.
    pub fn new_without_profile_with_policy(
        program: impl Into<Arc<Program>>,
        policy: ExecutionPolicy,
    ) -> Result<Self, ProgramValidationError> {
        Self::with_seed_without_profile_and_policy(program, DEFAULT_RNG_SEED, policy)
    }

    /// Creates a machine from a name-free execution image.
    ///
    /// # Errors
    /// Returns an error when `image` fails structural validation.
    pub fn new_execution_image(image: ExecutionImage) -> Result<Self, ProgramValidationError> {
        Self::new(image.into_program())
    }

    /// Creates a machine from an execution image with an explicit policy.
    ///
    /// # Errors
    /// Returns an error when `image` fails structural validation.
    pub fn new_execution_image_with_policy(
        image: ExecutionImage,
        policy: ExecutionPolicy,
    ) -> Result<Self, ProgramValidationError> {
        Self::new_with_policy(image.into_program(), policy)
    }

    /// Creates a seeded machine after validating `program`.
    ///
    /// # Errors
    /// Returns an error when `program` fails structural validation.
    pub fn with_seed(
        program: impl Into<Arc<Program>>,
        seed: i64,
    ) -> Result<Self, ProgramValidationError> {
        Self::with_seed_and_policy(program, seed, ExecutionPolicy::default())
    }

    /// Creates a seeded machine with an explicit policy.
    ///
    /// # Errors
    /// Returns an error when `program` fails structural validation.
    pub fn with_seed_and_policy(
        program: impl Into<Arc<Program>>,
        seed: i64,
        policy: ExecutionPolicy,
    ) -> Result<Self, ProgramValidationError> {
        let program = ValidatedProgram::new(program)?;
        Ok(Self::from_validated_with_seed_and_policy(
            &program, seed, policy,
        ))
    }

    /// Creates a seeded machine without allocating an execution profile.
    ///
    /// # Errors
    /// Returns an error when `program` fails structural validation.
    pub fn with_seed_without_profile(
        program: impl Into<Arc<Program>>,
        seed: i64,
    ) -> Result<Self, ProgramValidationError> {
        Self::with_seed_without_profile_and_policy(program, seed, ExecutionPolicy::default())
    }

    /// Creates a seeded machine with an explicit policy and no profile.
    ///
    /// # Errors
    /// Returns an error when `program` fails structural validation.
    pub fn with_seed_without_profile_and_policy(
        program: impl Into<Arc<Program>>,
        seed: i64,
        policy: ExecutionPolicy,
    ) -> Result<Self, ProgramValidationError> {
        let program = ValidatedProgram::new(program)?;
        Ok(Self::from_validated_with_seed_without_profile_and_policy(
            &program, seed, policy,
        ))
    }

    /// Creates a machine from bytecode that has already passed validation.
    #[must_use]
    pub fn from_validated(program: &ValidatedProgram) -> Self {
        Self::from_validated_with_seed(program, DEFAULT_RNG_SEED)
    }

    /// Creates a machine from validated bytecode with an explicit policy.
    #[must_use]
    pub fn from_validated_with_policy(program: &ValidatedProgram, policy: ExecutionPolicy) -> Self {
        Self::from_validated_with_seed_and_policy(program, DEFAULT_RNG_SEED, policy)
    }

    /// Creates a seeded machine without rescanning validated bytecode.
    #[must_use]
    pub fn from_validated_with_seed(program: &ValidatedProgram, seed: i64) -> Self {
        Self::from_validated_with_seed_and_policy(program, seed, ExecutionPolicy::default())
    }

    /// Creates a seeded machine from validated bytecode and an explicit policy.
    #[must_use]
    pub fn from_validated_with_seed_and_policy(
        program: &ValidatedProgram,
        seed: i64,
        policy: ExecutionPolicy,
    ) -> Self {
        Self::initialize(
            program.shared(),
            program.shared_execution_metadata(),
            seed,
            true,
            policy,
        )
    }

    /// Creates a seeded machine from validated bytecode without profiling.
    #[must_use]
    pub fn from_validated_with_seed_without_profile(program: &ValidatedProgram, seed: i64) -> Self {
        Self::from_validated_with_seed_without_profile_and_policy(
            program,
            seed,
            ExecutionPolicy::default(),
        )
    }

    /// Creates a seeded machine with an explicit policy and no profile.
    #[must_use]
    pub fn from_validated_with_seed_without_profile_and_policy(
        program: &ValidatedProgram,
        seed: i64,
        policy: ExecutionPolicy,
    ) -> Self {
        Self::initialize(
            program.shared(),
            program.shared_execution_metadata(),
            seed,
            false,
            policy,
        )
    }

    /// Creates a seeded machine by cloning a prevalidated initial frame.
    #[must_use]
    pub fn from_validated_with_seed_and_frame(
        program: &ValidatedProgram,
        seed: i64,
        initial: &InitialFrame,
    ) -> Option<Self> {
        Self::from_validated_with_seed_and_frame_and_policy(
            program,
            seed,
            initial,
            ExecutionPolicy::default(),
        )
    }

    /// Creates a machine from a prevalidated frame and explicit policy.
    #[must_use]
    pub fn from_validated_with_seed_and_frame_and_policy(
        program: &ValidatedProgram,
        seed: i64,
        initial: &InitialFrame,
        policy: ExecutionPolicy,
    ) -> Option<Self> {
        Self::from_validated_with_seed_and_frame_inner(program, seed, initial, true, policy)
    }

    /// Creates a seeded machine from an initial frame without profiling.
    #[must_use]
    pub fn from_validated_with_seed_and_frame_without_profile(
        program: &ValidatedProgram,
        seed: i64,
        initial: &InitialFrame,
    ) -> Option<Self> {
        Self::from_validated_with_seed_and_frame_without_profile_and_policy(
            program,
            seed,
            initial,
            ExecutionPolicy::default(),
        )
    }

    /// Creates a machine from an initial frame and policy without profiling.
    #[must_use]
    pub fn from_validated_with_seed_and_frame_without_profile_and_policy(
        program: &ValidatedProgram,
        seed: i64,
        initial: &InitialFrame,
        policy: ExecutionPolicy,
    ) -> Option<Self> {
        Self::from_validated_with_seed_and_frame_inner(program, seed, initial, false, policy)
    }
}
