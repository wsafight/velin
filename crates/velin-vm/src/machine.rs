//! The program executor: control flow, frames, and host-effect yielding.
//!
//! [`Machine`] runs a [`Program`] until it either finishes or reaches a host
//! effect. Host effects are the sole boundary to the outside world: on
//! [`Op::Host`] the machine evaluates the effect's arguments and returns
//! [`Yield::Host`] to the embedder, which performs the effect and calls
//! [`Machine::resume`] to continue. A loop that never yields is bounded by
//! [`MAX_IMMEDIATE_STEPS`] so a runaway script cannot hang the host.

use self::support::{cache_metrics, checked_total};
use crate::chunk::{FrameAccess, eval_validated_chunk};
use std::sync::Arc;
use velin_bytecode::{
    ExecutionImage, ExecutionMetadata, HostOp, InitialFrame, Op, Program, ProgramValidationError,
    UpdateOp, ValidatedProgram,
};
use velin_eval::EvalError;
use velin_syntax::{
    BinaryOp, DataFootprint, DataMetrics, MAX_DATA_DEPTH, MAX_DATA_TEXT_BYTES, MAX_DATA_VALUES,
    Value,
};

/// The maximum number of control-flow ops executed between two yields.
///
/// Mirrors the original runtime's guard against infinite loops. Reaching it is
/// reported as an error rather than hanging.
pub const MAX_IMMEDIATE_STEPS: usize = 10_000;

/// Maximum logical values retained across one machine frame.
pub const MAX_MACHINE_DATA_VALUES: usize = 100_000;

/// Maximum text retained across one machine frame.
pub const MAX_MACHINE_TEXT_BYTES: usize = 16 * 1024 * 1024;

/// Maximum logical values emitted in one host yield.
pub const MAX_HOST_PAYLOAD_VALUES: usize = 100_000;

/// Maximum text emitted in one host yield.
pub const MAX_HOST_PAYLOAD_TEXT_BYTES: usize = 16 * 1024 * 1024;

/// Seed used by [`Machine::new`] when a program contains random expressions.
pub const DEFAULT_RNG_SEED: i64 = 0;

/// Bounded execution counters suitable for an offline profile-guided compile.
/// Only validated program-counter IDs are recorded; values and host payloads
/// never enter the profile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutionProfile {
    op_hits: Box<[u64]>,
}

impl ExecutionProfile {
    fn new(op_count: usize) -> Self {
        Self {
            op_hits: vec![0; op_count].into_boxed_slice(),
        }
    }

    fn record(&mut self, pc: usize) {
        if let Some(hits) = self.op_hits.get_mut(pc) {
            *hits = hits.saturating_add(1);
        }
    }

    /// Returns per-op hit counters in program-counter order.
    #[must_use]
    pub fn op_hits(&self) -> &[u64] {
        &self.op_hits
    }

    /// Returns anonymous hot op IDs at or above `threshold` hits.
    #[must_use]
    pub fn hot_ops(&self, threshold: u64) -> Vec<(u32, u64)> {
        self.op_hits
            .iter()
            .enumerate()
            .filter_map(|(pc, hits)| {
                (*hits >= threshold)
                    .then(|| Some((u32::try_from(pc).ok()?, *hits)))
                    .flatten()
            })
            .collect()
    }

    /// Merges counters from another profile with the same program width.
    ///
    /// # Errors
    /// Returns an error if the profiles refer to different program widths.
    pub fn merge(&mut self, other: &Self) -> Result<(), &'static str> {
        if self.op_hits.len() != other.op_hits.len() {
            return Err("execution profiles refer to different program widths");
        }
        for (left, right) in self.op_hits.iter_mut().zip(&other.op_hits) {
            *left = left.saturating_add(*right);
        }
        Ok(())
    }
}

/// Why the machine stopped running.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Yield {
    /// A host effect must be performed. `values` are the evaluated arguments.
    /// The embedder handles `host_id` and calls [`Machine::resume`].
    Host { host_id: u32, values: Vec<Value> },
    /// The program halted normally.
    Finished,
}

/// A side-effect-only host command collected by [`Machine::run_effect_batch`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostEffect {
    /// The opaque host command identifier supplied by the compiled program.
    pub host_id: u32,
    /// Evaluated arguments owned by the host until it finishes the effect.
    pub values: Vec<Value>,
}

/// Why a host-provided variable could not be installed in a machine frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SetVariableError {
    UnknownVariable(String),
    InvalidValue(&'static str),
    StateBudget(&'static str),
}

impl std::fmt::Display for SetVariableError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownVariable(name) => write!(formatter, "unknown variable `{name}`"),
            Self::InvalidValue(message) | Self::StateBudget(message) => {
                formatter.write_str(message)
            }
        }
    }
}

impl std::error::Error for SetVariableError {}

/// A running program instance. Cloning it creates an in-memory checkpoint: the
/// program counter, variables, pending host request, and RNG slot are all
/// copied, so restoring the clone also rewinds future random draws.
#[derive(Debug, Clone)]
pub struct Machine {
    program: Arc<Program>,
    metadata: Arc<ExecutionMetadata>,
    frame: FrameState,
    frame_total: DataFootprint,
    register_values: Vec<Option<Value>>,
    register_metrics: Vec<Option<DataMetrics>>,
    effect_buffer: Vec<HostEffect>,
    profile: ExecutionProfile,
    pc: usize,
    /// The host effect execution is currently waiting to resume from.
    pending_host: Option<PendingHost>,
    finished: bool,
}

#[derive(Debug, Clone)]
struct FrameState {
    values: Vec<Option<Value>>,
    footprints: Vec<DataFootprint>,
    depths: Vec<u8>,
}

impl FrameState {
    fn metrics(&self, slot: usize) -> DataMetrics {
        DataMetrics {
            footprint: self.footprints[slot],
            max_depth: usize::from(self.depths[slot]),
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct PendingHost {
    bind: Option<u32>,
    line: usize,
}

impl Machine {
    /// Creates a machine for a validated `program` with an empty variable frame.
    ///
    /// # Errors
    /// Returns an error when serialized or manually assembled bytecode is
    /// malformed or exceeds a program budget.
    pub fn new(program: impl Into<Arc<Program>>) -> Result<Self, ProgramValidationError> {
        Self::with_seed(program, DEFAULT_RNG_SEED)
    }

    /// Creates a machine from a name-free runtime execution image.
    ///
    /// # Errors
    /// Returns an error when the execution image fails structural validation.
    pub fn new_execution_image(image: ExecutionImage) -> Result<Self, ProgramValidationError> {
        Self::new(image.into_program())
    }

    /// Creates a machine and initializes its threaded RNG slot with `seed`.
    ///
    /// # Errors
    /// Returns an error when `program` fails structural validation.
    pub fn with_seed(
        program: impl Into<Arc<Program>>,
        seed: i64,
    ) -> Result<Self, ProgramValidationError> {
        let program = ValidatedProgram::new(program)?;
        Ok(Self::from_validated_with_seed(&program, seed))
    }

    /// Creates a machine from a program that has already passed validation.
    #[must_use]
    pub fn from_validated(program: &ValidatedProgram) -> Self {
        Self::from_validated_with_seed(program, DEFAULT_RNG_SEED)
    }

    /// Creates a seeded machine without rescanning already validated bytecode.
    #[must_use]
    pub fn from_validated_with_seed(program: &ValidatedProgram, seed: i64) -> Self {
        Self::initialize(program.shared(), program.shared_execution_metadata(), seed)
    }

    /// Creates a seeded machine by cloning a prevalidated initial frame.
    ///
    /// Returns `None` if the frame width differs from the program or the
    /// aggregate machine-state budget would be exceeded after seeding RNG.
    #[must_use]
    pub fn from_validated_with_seed_and_frame(
        program: &ValidatedProgram,
        seed: i64,
        initial: &InitialFrame,
    ) -> Option<Self> {
        let metadata = program.shared_execution_metadata();
        let program = program.shared();
        let profile = ExecutionProfile::new(program.ops.len());
        if initial.values().len() != program.slots.len() {
            return None;
        }
        let mut frame = FrameState {
            values: initial.values().to_vec(),
            footprints: initial.footprints().to_vec(),
            depths: initial.depths().to_vec(),
        };
        let mut frame_total = initial.total();
        if let Some(slot) = program.slots.rng_state() {
            let index = slot as usize;
            let retained = DataFootprint {
                values: frame_total
                    .values
                    .checked_sub(frame.footprints[index].values)?,
                text_bytes: frame_total
                    .text_bytes
                    .checked_sub(frame.footprints[index].text_bytes)?,
            };
            let seed_metrics = DataMetrics {
                footprint: DataFootprint {
                    values: 1,
                    text_bytes: 0,
                },
                max_depth: 0,
            };
            frame_total = checked_total(
                retained,
                seed_metrics.footprint,
                MAX_MACHINE_DATA_VALUES,
                MAX_MACHINE_TEXT_BYTES,
                "machine state",
            )
            .ok()?;
            frame.values[index] = Some(Value::Integer(seed));
            cache_metrics(&mut frame, index, seed_metrics);
        } else {
            checked_total(
                DataFootprint::default(),
                frame_total,
                MAX_MACHINE_DATA_VALUES,
                MAX_MACHINE_TEXT_BYTES,
                "machine state",
            )
            .ok()?;
        }
        Some(Self {
            program,
            metadata,
            frame,
            frame_total,
            register_values: Vec::new(),
            register_metrics: Vec::new(),
            effect_buffer: Vec::new(),
            profile,
            pc: 0,
            pending_host: None,
            finished: false,
        })
    }

    fn initialize(program: Arc<Program>, metadata: Arc<ExecutionMetadata>, seed: i64) -> Self {
        let width = program.slots.len();
        let profile = ExecutionProfile::new(program.ops.len());
        let mut frame = vec![None; width];
        let mut frame_footprints = vec![DataFootprint::default(); width];
        let frame_depths = vec![0; width];
        let mut frame_total = DataFootprint::default();
        if let Some(slot) = program.slots.rng_state() {
            frame[slot as usize] = Some(Value::Integer(seed));
            frame_footprints[slot as usize] = DataFootprint {
                values: 1,
                text_bytes: 0,
            };
            frame_total.values = 1;
        }
        Self {
            program,
            metadata,
            frame: FrameState {
                values: frame,
                footprints: frame_footprints,
                depths: frame_depths,
            },
            frame_total,
            register_values: Vec::new(),
            register_metrics: Vec::new(),
            effect_buffer: Vec::new(),
            profile,
            pc: 0,
            pending_host: None,
            finished: false,
        }
    }

    /// Returns the immutable bytecode shared by this machine and its snapshots.
    #[must_use]
    pub fn program(&self) -> &Program {
        &self.program
    }

    /// Returns the anonymous execution profile accumulated by this machine.
    #[must_use]
    pub const fn profile(&self) -> &ExecutionProfile {
        &self.profile
    }

    /// Re-seeds the RNG state. Returns `false` when the program has no random
    /// expression and therefore no RNG slot.
    pub fn set_rng_seed(&mut self, seed: i64) -> bool {
        let Some(slot) = self.program.slots.rng_state() else {
            return false;
        };
        self.frame.values[slot as usize] = Some(Value::Integer(seed));
        true
    }

    /// Returns the current RNG state when the program contains random ops.
    #[must_use]
    pub fn rng_state(&self) -> Option<i64> {
        let slot = self.program.slots.rng_state()?;
        match self.frame.values[slot as usize].as_ref()? {
            Value::Integer(state) => Some(*state),
            _ => None,
        }
    }

    /// Presets a slot's value by name (e.g. for `default`-style initial state).
    ///
    /// Returns `false` if the program never referenced that name or `value`
    /// exceeds a per-value or aggregate machine data budget.
    pub fn set_variable(&mut self, name: &str, value: Value) -> bool {
        self.try_set_variable(name, value).is_ok()
    }

    /// Presets a variable and reports why the value could not be installed.
    ///
    /// # Errors
    /// Returns an error for an unknown name, an invalid value, or an aggregate
    /// machine-state budget violation.
    pub fn try_set_variable(&mut self, name: &str, value: Value) -> Result<(), SetVariableError> {
        let slot = self
            .program
            .slots
            .get(name)
            .ok_or_else(|| SetVariableError::UnknownVariable(name.to_owned()))?;
        let metrics = value
            .data_metrics()
            .map_err(SetVariableError::InvalidValue)?;
        self.replace_slot(slot, value, metrics)
            .map_err(SetVariableError::StateBudget)
    }

    /// Reads a slot's current value by name.
    #[must_use]
    pub fn variable(&self, name: &str) -> Option<&Value> {
        let slot = self.program.slots.get(name)?;
        self.frame.values[slot as usize].as_ref()
    }
}

mod batch;
mod execution;
mod support;
mod updates;

#[cfg(test)]
#[path = "machine_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "restart_tests.rs"]
mod restart_tests;

#[cfg(test)]
#[path = "batch_tests.rs"]
mod batch_tests;

#[cfg(test)]
#[path = "register_tests.rs"]
mod register_tests;
