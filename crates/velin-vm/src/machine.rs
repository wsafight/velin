//! The program executor: control flow, frames, and host-effect yielding.
//!
//! [`Machine`] runs a [`Program`] until it either finishes or reaches a host
//! effect. Host effects are the sole boundary to the outside world: on
//! [`Op::Host`] the machine evaluates the effect's arguments and returns
//! [`Yield::Host`] to the embedder, which performs the effect and calls
//! [`Machine::resume`] to continue. A loop that never yields is bounded by
//! [`MAX_IMMEDIATE_STEPS`] so a runaway script cannot hang the host.

use self::invoker::length_guards;
use self::support::{cache_metrics, checked_total};
use crate::chunk::{FrameAccess, eval_validated_chunk};
use std::sync::Arc;
use velin_bytecode::{
    ExecutionMetadata, HostOp, InitialFrame, Op, Program, UpdateOp, ValidatedProgram,
};
use velin_eval::EvalError;
use velin_syntax::{BinaryOp, DataFootprint, DataMetrics, MAX_DATA_DEPTH, Value};

mod budget;
mod invoker;
mod policy;
mod types;
pub use invoker::MachineInvoker;
pub use policy::{DEFAULT_MAX_FUEL, ExecutionPolicy, ExecutionProgress, ProgressCallback};
pub use types::{ExecutionProfile, FastYield, HostEffect, SetVariableError, Yield};

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

/// A running program instance. Cloning it creates an in-memory checkpoint: the
/// program counter, variables, pending host request, and RNG slot are all
/// copied, so restoring the clone also rewinds future random draws.
#[derive(Debug, Clone)]
pub struct Machine {
    program: Arc<Program>,
    metadata: Arc<ExecutionMetadata>,
    policy: ExecutionPolicy,
    frame: FrameState,
    frame_total: DataFootprint,
    register_values: Vec<Option<Value>>,
    register_metrics: Vec<Option<DataMetrics>>,
    register_touched: Vec<usize>,
    effect_buffer: Vec<HostEffect>,
    length_guards: Box<[Option<LengthGuard>]>,
    profile: ExecutionProfile,
    pc: usize,
    /// The host effect execution is currently waiting to resume from.
    pending_host: Option<PendingHost>,
    /// An execution error reached after a batch already collected effects.
    /// The error is reported after those effects have been drained.
    pending_batch_error: Option<EvalError>,
    finished: bool,
    fuel_used: u64,
    host_effects: usize,
    call_depth: usize,
    cancelled: bool,
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

#[derive(Debug, Clone, Copy)]
struct LengthGuard {
    index_slot: u32,
    collection_slot: u32,
    comparison: BinaryOp,
}

impl Machine {
    fn from_validated_with_seed_and_frame_inner(
        program: &ValidatedProgram,
        seed: i64,
        initial: &InitialFrame,
        profile_enabled: bool,
        policy: ExecutionPolicy,
    ) -> Option<Self> {
        let metadata = program.shared_execution_metadata();
        let program = program.shared();
        let profile = if profile_enabled {
            ExecutionProfile::new(program.ops.len())
        } else {
            ExecutionProfile::disabled()
        };
        let length_guards = length_guards(&program);
        if !initial.matches_layout(&program.slots) {
            return None;
        }
        let mut frame = FrameState {
            values: initial.values().to_vec(),
            footprints: initial.footprints().to_vec(),
            depths: initial.depths().to_vec(),
        };
        let mut frame_total = initial.total();
        if frame.footprints.iter().any(|footprint| {
            footprint.values > policy.max_value_values
                || footprint.text_bytes > policy.max_value_text_bytes
        }) {
            return None;
        }
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
                policy.max_machine_values,
                policy.max_machine_text_bytes,
                "machine state",
            )
            .ok()?;
            frame.values[index] = Some(Value::Integer(seed));
            cache_metrics(&mut frame, index, seed_metrics);
        } else {
            checked_total(
                DataFootprint::default(),
                frame_total,
                policy.max_machine_values,
                policy.max_machine_text_bytes,
                "machine state",
            )
            .ok()?;
        }
        Some(Self {
            program,
            metadata,
            policy,
            frame,
            frame_total,
            register_values: Vec::new(),
            register_metrics: Vec::new(),
            register_touched: Vec::new(),
            effect_buffer: Vec::new(),
            length_guards,
            profile,
            pc: 0,
            pending_host: None,
            pending_batch_error: None,
            finished: false,
            fuel_used: 0,
            host_effects: 0,
            call_depth: 1,
            cancelled: false,
        })
    }

    fn initialize(
        program: Arc<Program>,
        metadata: Arc<ExecutionMetadata>,
        seed: i64,
        profile_enabled: bool,
        policy: ExecutionPolicy,
    ) -> Self {
        let width = program.slots.len();
        let profile = if profile_enabled {
            ExecutionProfile::new(program.ops.len())
        } else {
            ExecutionProfile::disabled()
        };
        let length_guards = length_guards(&program);
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
            policy,
            frame: FrameState {
                values: frame,
                footprints: frame_footprints,
                depths: frame_depths,
            },
            frame_total,
            register_values: Vec::new(),
            register_metrics: Vec::new(),
            register_touched: Vec::new(),
            effect_buffer: Vec::new(),
            length_guards,
            profile,
            pc: 0,
            pending_host: None,
            pending_batch_error: None,
            finished: false,
            fuel_used: 0,
            host_effects: 0,
            call_depth: 1,
            cancelled: false,
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

    /// Clears all recorded instruction hits while retaining the profile
    /// allocation. Disabled profiles remain disabled.
    pub fn reset_profile(&mut self) {
        self.profile.reset();
    }

    /// Enables instruction hit recording, allocating counters for this
    /// machine's program if necessary.
    pub fn enable_profile(&mut self) {
        if !self.profile.enabled() {
            self.profile = ExecutionProfile::new(self.program.ops.len());
        }
    }

    /// Disables instruction hit recording and releases its counters.
    pub fn disable_profile(&mut self) {
        self.profile = ExecutionProfile::disabled();
    }

    /// Returns whether this machine records execution profile hits.
    #[must_use]
    pub const fn profile_enabled(&self) -> bool {
        self.profile.enabled()
    }

    /// Re-seeds the RNG state. Returns `false` when the program has no random
    /// expression and therefore no RNG slot.
    pub fn set_rng_seed(&mut self, seed: i64) -> bool {
        let Some(slot) = self.program.slots.rng_state() else {
            return false;
        };
        self.replace_slot(
            slot,
            Value::Integer(seed),
            DataMetrics {
                footprint: DataFootprint {
                    values: 1,
                    text_bytes: 0,
                },
                max_depth: 0,
            },
        )
        .is_ok()
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
        self.try_set_slot(slot, value)
    }

    /// Presets a variable by its pre-resolved dense frame slot.
    ///
    /// Hosts that bind the same compiled program repeatedly can resolve names
    /// once and avoid a string lookup on every invocation.
    ///
    /// # Errors
    /// Returns an error for an invalid slot, value, or aggregate machine-state
    /// budget violation.
    pub fn try_set_slot(&mut self, slot: u32, value: Value) -> Result<(), SetVariableError> {
        let index = slot as usize;
        if index >= self.frame.values.len() {
            return Err(SetVariableError::UnknownSlot(slot));
        }
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
mod fast;
mod guards;
mod support;
mod updates;

#[cfg(test)]
#[path = "machine_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "machine_error_tests.rs"]
mod error_tests;

#[cfg(test)]
#[path = "restart_tests.rs"]
mod restart_tests;

#[cfg(test)]
#[path = "batch_tests.rs"]
mod batch_tests;

#[cfg(test)]
#[path = "register_tests.rs"]
mod register_tests;

#[cfg(test)]
#[path = "coverage_tests.rs"]
mod coverage_tests;

#[cfg(test)]
#[path = "coverage_more_tests.rs"]
mod coverage_more_tests;
