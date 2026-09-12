//! The program executor: control flow, frames, and host-effect yielding.
//!
//! [`Machine`] runs a [`Program`] until it either finishes or reaches a host
//! effect. Host effects are the sole boundary to the outside world: on
//! [`Op::Host`] the machine evaluates the effect's arguments and returns
//! [`Yield::Host`] to the embedder, which performs the effect and calls
//! [`Machine::resume`] to continue. A loop that never yields is bounded by
//! [`MAX_IMMEDIATE_STEPS`] so a runaway script cannot hang the host.

use crate::chunk::{FrameAccess, eval_validated_chunk};
use std::sync::Arc;
use velin_compile::{
    ExecutionMetadata, InitialFrame, Op, Program, ProgramValidationError, QuickenedCallRef,
    QuickenedOperand, UpdateOp, ValidatedProgram,
};
use velin_eval::{EvalError, invoke_readonly_measured, invoke_stack_measured};
use velin_syntax::{
    BinaryOp, Builtin, DataFootprint, DataMetrics, MAX_DATA_DEPTH, MAX_DATA_TEXT_BYTES,
    MAX_DATA_VALUES, Value,
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

/// Why the machine stopped running.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Yield {
    /// A host effect must be performed. `values` are the evaluated arguments.
    /// The embedder handles `host_id` and calls [`Machine::resume`].
    Host { host_id: u32, values: Vec<Value> },
    /// The program halted normally.
    Finished,
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
    expression_stack: Vec<Value>,
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
            expression_stack: Vec::new(),
            pc: 0,
            pending_host: None,
            finished: false,
        })
    }

    fn initialize(program: Arc<Program>, metadata: Arc<ExecutionMetadata>, seed: i64) -> Self {
        let width = program.slots.len();
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
            expression_stack: Vec::new(),
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

    /// Runs until the program yields a host effect or finishes.
    ///
    /// # Errors
    /// Returns [`EvalError`] from any expression evaluation, or a synthetic
    /// error if [`MAX_IMMEDIATE_STEPS`] is exceeded.
    pub fn run(&mut self) -> Result<Yield, EvalError> {
        if let Some(pending) = self.pending_host {
            return Err(EvalError::new(
                pending.line,
                "machine is waiting for the host; call `resume`",
            ));
        }
        let mut steps = 0;
        while !self.finished {
            steps += 1;
            if steps > MAX_IMMEDIATE_STEPS {
                return Err(EvalError::new(
                    self.current_line(),
                    "possible infinite loop: too many steps without yielding",
                ));
            }
            if self.pc >= self.program.ops.len() {
                self.finished = true;
                break;
            }
            if let Some(effect) = self.step()? {
                return Ok(effect);
            }
        }
        Ok(Yield::Finished)
    }

    /// Resumes after a [`Yield::Host`]. A host op with a `bind` destination
    /// requires `Some(value)`; side-effect-only host ops ignore a supplied value.
    ///
    /// # Errors
    /// Propagates evaluation errors from continued execution.
    pub fn resume(&mut self, value: Option<Value>) -> Result<Yield, EvalError> {
        let pending = self.pending_host.ok_or_else(|| {
            EvalError::new(
                self.current_line(),
                "cannot resume: no host effect is pending",
            )
        })?;
        if let Some(slot) = pending.bind {
            let value = value.ok_or_else(|| {
                EvalError::new(pending.line, "bound host effect returned no value")
            })?;
            self.assign(slot, value, pending.line)?;
        }
        self.pending_host = None;
        self.run()
    }

    /// Executes one op. Returns `Some(Yield::Host)` if it yielded, `None`
    /// otherwise. Advances `pc` accordingly.
    #[allow(clippy::too_many_lines)]
    fn step(&mut self) -> Result<Option<Yield>, EvalError> {
        match &self.program.ops[self.pc] {
            Op::Set { slot, value } => {
                let slot = *slot;
                let value = *value;
                let (result, metrics) = eval_chunk_for(
                    &self.program,
                    &self.metadata,
                    &mut self.frame,
                    &mut self.expression_stack,
                    value,
                )?;
                let line = self.program.chunks[value as usize].line as usize;
                self.assign_measured(slot, result, metrics, line)?;
                self.pc += 1;
            }
            Op::SetConst { slot, value, line } => {
                let slot = *slot;
                let value = value.clone();
                let line = *line as usize;
                let metrics = self
                    .metadata
                    .op_constant_metrics(self.pc)
                    .expect("validated constant metadata");
                self.assign_measured(slot, value, metrics, line)?;
                self.pc += 1;
            }
            Op::CopySlot {
                slot, source, line, ..
            } => {
                let slot = *slot;
                let source = *source;
                let line = *line as usize;
                self.require_assigned(source, line)?;
                if slot != source {
                    let value = self.frame.values[source as usize]
                        .as_ref()
                        .expect("copy source was checked")
                        .clone();
                    let metrics = self.frame.metrics(source as usize);
                    self.assign_measured(slot, value, metrics, line)?;
                }
                self.pc += 1;
            }
            Op::Update {
                slot,
                operation,
                line,
                ..
            } => {
                let slot = *slot;
                let operation = *operation;
                let line = *line as usize;
                self.step_update(slot, operation, line)?;
                self.pc += 1;
            }
            Op::Jump(target) => self.pc = *target as usize,
            Op::JumpIfFalse { condition, target } => {
                let condition_line = self.program.chunks[*condition as usize].line as usize;
                let condition_value = eval_chunk_for(
                    &self.program,
                    &self.metadata,
                    &mut self.frame,
                    &mut self.expression_stack,
                    *condition,
                )?
                .0;
                match condition_value {
                    Value::Boolean(false) => self.pc = *target as usize,
                    Value::Boolean(true) => self.pc += 1,
                    value => {
                        return Err(EvalError::new(
                            condition_line,
                            format!("condition expects boolean, found {}", value.type_name()),
                        ));
                    }
                }
            }
            Op::JumpIfIntegerCompare {
                condition,
                slot,
                comparison,
                value,
                target,
            } => {
                let line = self.program.chunks[*condition as usize].line as usize;
                self.require_assigned(*slot, line)?;
                let source = self.frame.values[*slot as usize]
                    .as_ref()
                    .expect("comparison source was checked");
                let result = compare_integer_literal(source, *comparison, *value, line)?;
                if result {
                    self.pc += 1;
                } else {
                    self.pc = *target as usize;
                }
            }
            Op::Host(host) => {
                let host_id = host.host_id;
                let bind = host.bind;
                let line = host.line as usize;
                let mut values = Vec::with_capacity(host.args.len());
                let mut payload = DataFootprint::default();
                for chunk in host.args.iter().copied() {
                    let (value, metrics) = eval_chunk_for(
                        &self.program,
                        &self.metadata,
                        &mut self.frame,
                        &mut self.expression_stack,
                        chunk,
                    )?;
                    payload = checked_total(
                        payload,
                        metrics.footprint,
                        MAX_HOST_PAYLOAD_VALUES,
                        MAX_HOST_PAYLOAD_TEXT_BYTES,
                        "host payload",
                    )
                    .map_err(|error| EvalError::new(line, error))?;
                    values.push(value);
                }
                self.pc += 1; // resume past the effect, never re-run it
                self.pending_host = Some(PendingHost { bind, line });
                return Ok(Some(Yield::Host { host_id, values }));
            }
            Op::Halt => self.finished = true,
        }
        Ok(None)
    }

    fn step_update(
        &mut self,
        slot: u32,
        operation: UpdateOp,
        line: usize,
    ) -> Result<(), EvalError> {
        if let UpdateOp::AddInteger { value } = operation {
            return self.update_add_integer(slot, value, line);
        }
        self.require_assigned(slot, line)?;
        match operation {
            UpdateOp::Add { rhs } => {
                let (rhs, _) = eval_chunk_for(
                    &self.program,
                    &self.metadata,
                    &mut self.frame,
                    &mut self.expression_stack,
                    rhs,
                )?;
                self.update_add(slot, rhs, line)
            }
            UpdateOp::AddInteger { .. } => unreachable!("handled before expression updates"),
            UpdateOp::Push { value } => {
                let (value, metrics) = eval_chunk_for(
                    &self.program,
                    &self.metadata,
                    &mut self.frame,
                    &mut self.expression_stack,
                    value,
                )?;
                self.update_push(slot, value, metrics, line)
            }
            UpdateOp::Put { key, value } => {
                let (key, _) = eval_chunk_for(
                    &self.program,
                    &self.metadata,
                    &mut self.frame,
                    &mut self.expression_stack,
                    key,
                )?;
                let (value, metrics) = eval_chunk_for(
                    &self.program,
                    &self.metadata,
                    &mut self.frame,
                    &mut self.expression_stack,
                    value,
                )?;
                self.update_put(slot, key, value, metrics, line)
            }
            UpdateOp::Remove { key } => {
                let (key, _) = eval_chunk_for(
                    &self.program,
                    &self.metadata,
                    &mut self.frame,
                    &mut self.expression_stack,
                    key,
                )?;
                self.update_remove(slot, key, line)
            }
        }
    }

    fn update_add_integer(&mut self, slot: u32, value: i64, line: usize) -> Result<(), EvalError> {
        let index = slot as usize;
        let slot_name = self.program.slots.name(slot).unwrap_or("?");
        let frame = &mut self.frame;
        let Some(source) = frame.values[index].as_ref() else {
            return Err(velin_eval::unassigned(line, slot_name));
        };
        let Value::Integer(source) = source else {
            return Err(EvalError::new(
                line,
                format!("`+` cannot combine {} and integer", source.type_name()),
            ));
        };
        let result = source
            .checked_add(value)
            .ok_or_else(|| EvalError::new(line, "integer overflow"))?;
        let footprint = DataFootprint {
            values: 1,
            text_bytes: 0,
        };
        let old = frame.footprints[index];
        let retained = DataFootprint {
            values: self.frame_total.values - old.values,
            text_bytes: self.frame_total.text_bytes - old.text_bytes,
        };
        let total = checked_total(
            retained,
            footprint,
            MAX_MACHINE_DATA_VALUES,
            MAX_MACHINE_TEXT_BYTES,
            "machine state",
        )
        .map_err(|error| EvalError::new(line, error))?;
        frame.values[index] = Some(Value::Integer(result));
        cache_metrics(
            frame,
            index,
            DataMetrics {
                footprint,
                max_depth: 0,
            },
        );
        self.frame_total = total;
        Ok(())
    }

    fn require_assigned(&self, slot: u32, line: usize) -> Result<(), EvalError> {
        if self.frame.values[slot as usize].is_some() {
            return Ok(());
        }
        Err(velin_eval::unassigned(
            line,
            self.program.slots.name(slot).unwrap_or("?"),
        ))
    }

    fn update_add(&mut self, slot: u32, rhs: Value, line: usize) -> Result<(), EvalError> {
        let source = self.frame.values[slot as usize]
            .as_ref()
            .expect("update source was checked");
        let footprint = match (source, &rhs) {
            (Value::Integer(left), Value::Integer(right)) => {
                left.checked_add(*right)
                    .ok_or_else(|| EvalError::new(line, "integer overflow"))?;
                DataFootprint {
                    values: 1,
                    text_bytes: 0,
                }
            }
            (Value::String(left), Value::String(right)) => {
                let text_bytes = left
                    .len()
                    .checked_add(right.len())
                    .ok_or_else(|| EvalError::new(line, "data text exceeds 1 MiB"))?;
                if text_bytes > MAX_DATA_TEXT_BYTES {
                    return Err(EvalError::new(line, "data text exceeds 1 MiB"));
                }
                DataFootprint {
                    values: 1,
                    text_bytes,
                }
            }
            _ => {
                return Err(EvalError::new(
                    line,
                    format!(
                        "`+` cannot combine {} and {}",
                        source.type_name(),
                        rhs.type_name()
                    ),
                ));
            }
        };
        let total = self.replacement_total(slot, footprint, line)?;
        let source = self.frame.values[slot as usize]
            .take()
            .expect("update source was checked");
        let result = match (source, rhs) {
            (Value::Integer(left), Value::Integer(right)) => {
                Value::Integer(left.checked_add(right).expect("addition was preflighted"))
            }
            (Value::String(mut left), Value::String(right)) => {
                left.make_mut().push_str(&right);
                Value::String(left)
            }
            _ => unreachable!("update types were preflighted"),
        };
        self.install_prechecked(
            slot,
            result,
            DataMetrics {
                footprint,
                max_depth: 0,
            },
            total,
        );
        Ok(())
    }

    fn update_push(
        &mut self,
        slot: u32,
        value: Value,
        metrics: DataMetrics,
        line: usize,
    ) -> Result<(), EvalError> {
        let Value::List(_) = self.frame.values[slot as usize]
            .as_ref()
            .expect("update source was checked")
        else {
            return Err(EvalError::new(line, "push expects a list"));
        };
        ensure_child_depth(metrics, line)?;
        let current = self.frame.metrics(slot as usize);
        let footprint = adjusted_footprint(
            current.footprint,
            DataFootprint::default(),
            metrics.footprint,
            0,
            0,
            line,
        )?;
        let total = self.replacement_total(slot, footprint, line)?;
        let Value::List(mut values) = self.frame.values[slot as usize]
            .take()
            .expect("update source was checked")
        else {
            unreachable!("update type was preflighted")
        };
        Arc::make_mut(&mut values).push(value);
        self.install_prechecked(
            slot,
            Value::List(values),
            DataMetrics {
                footprint,
                max_depth: current.max_depth.max(metrics.max_depth + 1),
            },
            total,
        );
        Ok(())
    }

    fn update_put(
        &mut self,
        slot: u32,
        key: Value,
        value: Value,
        metrics: DataMetrics,
        line: usize,
    ) -> Result<(), EvalError> {
        ensure_child_depth(metrics, line)?;
        let source = self.frame.values[slot as usize]
            .as_ref()
            .expect("update source was checked");
        let (removed, removed_key_bytes, added_key_bytes) = match (source, &key) {
            (Value::List(values), Value::Integer(index)) => {
                let index = usize::try_from(*index)
                    .ok()
                    .filter(|index| *index < values.len())
                    .ok_or_else(|| EvalError::new(line, "list index out of bounds"))?;
                (Some(value_metrics(&values[index], line)?), 0, 0)
            }
            (Value::Record(values), Value::String(key)) => match values.get(key.as_str()) {
                Some(previous) => (Some(value_metrics(previous, line)?), 0, 0),
                None => (None, 0, key.len()),
            },
            _ => {
                return Err(EvalError::new(
                    line,
                    "put/remove expects a list and integer index, or a record and string key",
                ));
            }
        };
        let current = self.frame.metrics(slot as usize);
        let footprint = adjusted_footprint(
            current.footprint,
            removed.map_or(DataFootprint::default(), |metrics| metrics.footprint),
            metrics.footprint,
            removed_key_bytes,
            added_key_bytes,
            line,
        )?;
        let total = self.replacement_total(slot, footprint, line)?;
        let source = self.frame.values[slot as usize]
            .take()
            .expect("update source was checked");
        let result = match (source, key) {
            (Value::List(mut values), Value::Integer(index)) => {
                let index = usize::try_from(index).expect("list index was preflighted");
                Arc::make_mut(&mut values)[index] = value;
                Value::List(values)
            }
            (Value::Record(mut values), Value::String(key)) => {
                Arc::make_mut(&mut values).insert(key.into_string(), value);
                Value::Record(values)
            }
            _ => unreachable!("update types were preflighted"),
        };
        let max_depth = updated_collection_depth(current, removed, Some(metrics), &result, line)?;
        self.install_prechecked(
            slot,
            result,
            DataMetrics {
                footprint,
                max_depth,
            },
            total,
        );
        Ok(())
    }

    fn update_remove(&mut self, slot: u32, key: Value, line: usize) -> Result<(), EvalError> {
        let source = self.frame.values[slot as usize]
            .as_ref()
            .expect("update source was checked");
        let (removed, removed_key_bytes) = match (source, &key) {
            (Value::List(values), Value::Integer(index)) => {
                let index = usize::try_from(*index)
                    .ok()
                    .filter(|index| *index < values.len())
                    .ok_or_else(|| EvalError::new(line, "list index out of bounds"))?;
                (value_metrics(&values[index], line)?, 0)
            }
            (Value::Record(values), Value::String(key)) => {
                let previous = values
                    .get(key.as_str())
                    .ok_or_else(|| EvalError::new(line, "missing record key"))?;
                (value_metrics(previous, line)?, key.len())
            }
            _ => {
                return Err(EvalError::new(
                    line,
                    "put/remove expects a list and integer index, or a record and string key",
                ));
            }
        };
        let current = self.frame.metrics(slot as usize);
        let footprint = adjusted_footprint(
            current.footprint,
            removed.footprint,
            DataFootprint::default(),
            removed_key_bytes,
            0,
            line,
        )?;
        let total = self.replacement_total(slot, footprint, line)?;
        let source = self.frame.values[slot as usize]
            .take()
            .expect("update source was checked");
        let result = match (source, key) {
            (Value::List(mut values), Value::Integer(index)) => {
                let index = usize::try_from(index).expect("list index was preflighted");
                Arc::make_mut(&mut values).remove(index);
                Value::List(values)
            }
            (Value::Record(mut values), Value::String(key)) => {
                Arc::make_mut(&mut values).remove(key.as_str());
                Value::Record(values)
            }
            _ => unreachable!("update types were preflighted"),
        };
        let max_depth = updated_collection_depth(current, Some(removed), None, &result, line)?;
        self.install_prechecked(
            slot,
            result,
            DataMetrics {
                footprint,
                max_depth,
            },
            total,
        );
        Ok(())
    }

    fn replacement_total(
        &self,
        slot: u32,
        footprint: DataFootprint,
        line: usize,
    ) -> Result<DataFootprint, EvalError> {
        let old = self.frame.metrics(slot as usize).footprint;
        let retained = DataFootprint {
            values: self.frame_total.values - old.values,
            text_bytes: self.frame_total.text_bytes - old.text_bytes,
        };
        checked_total(
            retained,
            footprint,
            MAX_MACHINE_DATA_VALUES,
            MAX_MACHINE_TEXT_BYTES,
            "machine state",
        )
        .map_err(|error| EvalError::new(line, error))
    }

    fn install_prechecked(
        &mut self,
        slot: u32,
        value: Value,
        metrics: DataMetrics,
        total: DataFootprint,
    ) {
        let index = slot as usize;
        let frame = &mut self.frame;
        frame.values[index] = Some(value);
        cache_metrics(frame, index, metrics);
        self.frame_total = total;
    }

    fn assign(&mut self, slot: u32, value: Value, line: usize) -> Result<(), EvalError> {
        let metrics = value
            .data_metrics()
            .map_err(|error| EvalError::new(line, error))?;
        self.assign_measured(slot, value, metrics, line)
    }

    fn assign_measured(
        &mut self,
        slot: u32,
        value: Value,
        metrics: DataMetrics,
        line: usize,
    ) -> Result<(), EvalError> {
        self.replace_slot(slot, value, metrics)
            .map_err(|error| EvalError::new(line, error))
    }

    fn replace_slot(
        &mut self,
        slot: u32,
        value: Value,
        metrics: DataMetrics,
    ) -> Result<(), &'static str> {
        let index = slot as usize;
        let frame = &mut self.frame;
        let old = frame.footprints[index];
        if old == metrics.footprint {
            frame.values[index] = Some(value);
            frame.depths[index] = u8::try_from(metrics.max_depth)
                .expect("validated value depth fits in the compact frame cache");
            return Ok(());
        }
        let retained = DataFootprint {
            values: self.frame_total.values - old.values,
            text_bytes: self.frame_total.text_bytes - old.text_bytes,
        };
        let total = checked_total(
            retained,
            metrics.footprint,
            MAX_MACHINE_DATA_VALUES,
            MAX_MACHINE_TEXT_BYTES,
            "machine state",
        )?;
        frame.values[index] = Some(value);
        cache_metrics(frame, index, metrics);
        self.frame_total = total;
        Ok(())
    }

    fn current_line(&self) -> usize {
        self.metadata
            .op(self.pc)
            .map_or(0, |metadata| metadata.line as usize)
    }
}

fn compare_integer_literal(
    source: &Value,
    comparison: BinaryOp,
    value: i64,
    line: usize,
) -> Result<bool, EvalError> {
    let Value::Integer(source) = source else {
        return match comparison {
            BinaryOp::Equal => Ok(false),
            BinaryOp::NotEqual => Ok(true),
            _ => Err(EvalError::new(
                line,
                format!(
                    "comparison cannot combine {} and integer",
                    source.type_name()
                ),
            )),
        };
    };
    Ok(match comparison {
        BinaryOp::Equal => *source == value,
        BinaryOp::NotEqual => *source != value,
        BinaryOp::Less => *source < value,
        BinaryOp::LessEqual => *source <= value,
        BinaryOp::Greater => *source > value,
        BinaryOp::GreaterEqual => *source >= value,
        _ => unreachable!("validated integer comparison"),
    })
}

fn cache_metrics(frame: &mut FrameState, slot: usize, metrics: DataMetrics) {
    frame.footprints[slot] = metrics.footprint;
    frame.depths[slot] = u8::try_from(metrics.max_depth)
        .expect("validated value depth fits in the compact frame cache");
}

fn eval_chunk_for(
    program: &Program,
    metadata: &ExecutionMetadata,
    frame: &mut FrameState,
    expression_stack: &mut Vec<Value>,
    chunk_id: u32,
) -> Result<(Value, DataMetrics), EvalError> {
    let (execution, result_metrics, quickened) = metadata
        .chunk_plan(chunk_id)
        .expect("validated chunk metadata");
    if let Some(call) = quickened {
        return eval_quickened_call(program, frame, expression_stack, call);
    }
    let chunk = program
        .chunk(chunk_id)
        .expect("program expression range was validated");
    let (frame, frame_metrics) = if execution.mutates_frame {
        let frame_metrics = execution
            .inherits_slot_metrics
            .then_some((frame.footprints.as_slice(), frame.depths.as_slice()));
        (
            FrameAccess::Mutable(frame.values.as_mut_slice()),
            frame_metrics,
        )
    } else {
        let frame_metrics = execution
            .inherits_slot_metrics
            .then_some((frame.footprints.as_slice(), frame.depths.as_slice()));
        (
            FrameAccess::ReadOnly(frame.values.as_slice()),
            frame_metrics,
        )
    };
    let slots = &program.slots;
    eval_validated_chunk(
        chunk,
        frame,
        frame_metrics,
        expression_stack,
        execution.max_stack as usize,
        result_metrics,
        |slot| slots.name(slot).unwrap_or("?").to_owned(),
    )
}

#[allow(clippy::option_as_ref_cloned)]
fn eval_quickened_call(
    program: &Program,
    frame: &FrameState,
    stack: &mut Vec<Value>,
    call: QuickenedCallRef<'_>,
) -> Result<(Value, DataMetrics), EvalError> {
    let line = call.line as usize;
    stack.clear();
    if let Some(result) = eval_quickened_readonly_call(program, frame, call)? {
        return Ok(result);
    }
    if stack.capacity() < call.operands.len() {
        stack.reserve(call.operands.len() - stack.capacity());
    }
    for operand in call.operands {
        let value = match operand {
            QuickenedOperand::Constant(index) => program.constants[*index as usize].clone(),
            QuickenedOperand::Slot(slot) => frame.values[*slot as usize]
                .as_ref()
                .cloned()
                .ok_or_else(|| {
                    velin_eval::unassigned(line, program.slots.name(*slot).unwrap_or("?"))
                })?,
        };
        stack.push(value);
    }
    let metrics = invoke_stack_measured(call.function, stack, call.operands.len(), line)?;
    let result = stack
        .pop()
        .expect("validated quickened built-in produced a result");
    debug_assert!(stack.is_empty());
    Ok((result, metrics))
}

fn eval_quickened_readonly_call(
    program: &Program,
    frame: &FrameState,
    call: QuickenedCallRef<'_>,
) -> Result<Option<(Value, DataMetrics)>, EvalError> {
    if !matches!(
        call.function,
        Builtin::Len | Builtin::Get | Builtin::Contains
    ) {
        return Ok(None);
    }
    let line = call.line as usize;
    let resolve =
        |operand: &QuickenedOperand| resolve_quickened_operand(program, frame, *operand, line);
    let result = match call.operands {
        [first] => {
            let arguments = [resolve(first)?];
            invoke_readonly_measured(call.function, &arguments, line)?
        }
        [first, second] => {
            let arguments = [resolve(first)?, resolve(second)?];
            invoke_readonly_measured(call.function, &arguments, line)?
        }
        [first, second, third] => {
            let arguments = [resolve(first)?, resolve(second)?, resolve(third)?];
            invoke_readonly_measured(call.function, &arguments, line)?
        }
        _ => unreachable!("validated read-only built-in arity"),
    };
    Ok(Some(result))
}

fn resolve_quickened_operand<'a>(
    program: &'a Program,
    frame: &'a FrameState,
    operand: QuickenedOperand,
    line: usize,
) -> Result<&'a Value, EvalError> {
    match operand {
        QuickenedOperand::Constant(index) => Ok(&program.constants[index as usize]),
        QuickenedOperand::Slot(slot) => frame.values[slot as usize]
            .as_ref()
            .ok_or_else(|| velin_eval::unassigned(line, program.slots.name(slot).unwrap_or("?"))),
    }
}

fn value_metrics(value: &Value, line: usize) -> Result<DataMetrics, EvalError> {
    value
        .data_metrics()
        .map_err(|error| EvalError::new(line, error))
}

fn ensure_child_depth(metrics: DataMetrics, line: usize) -> Result<(), EvalError> {
    if metrics.max_depth >= MAX_DATA_DEPTH {
        return Err(EvalError::new(
            line,
            "data exceeds 4096 values or 16 nesting levels",
        ));
    }
    Ok(())
}

fn updated_collection_depth(
    current: DataMetrics,
    removed: Option<DataMetrics>,
    added: Option<DataMetrics>,
    result: &Value,
    line: usize,
) -> Result<usize, EvalError> {
    let added_depth = added.map_or(0, |metrics| metrics.max_depth + 1);
    let removed_was_deepest =
        removed.is_some_and(|metrics| metrics.max_depth + 1 == current.max_depth);
    if removed_was_deepest && added_depth < current.max_depth {
        return value_metrics(result, line).map(|metrics| metrics.max_depth);
    }
    Ok(current.max_depth.max(added_depth))
}

fn adjusted_footprint(
    current: DataFootprint,
    removed: DataFootprint,
    added: DataFootprint,
    removed_key_bytes: usize,
    added_key_bytes: usize,
    line: usize,
) -> Result<DataFootprint, EvalError> {
    let values = current
        .values
        .checked_sub(removed.values)
        .and_then(|values| values.checked_add(added.values))
        .ok_or_else(|| EvalError::new(line, "data value count overflow"))?;
    if values > MAX_DATA_VALUES {
        return Err(EvalError::new(
            line,
            "data exceeds 4096 values or 16 nesting levels",
        ));
    }
    let removed_text = removed
        .text_bytes
        .checked_add(removed_key_bytes)
        .ok_or_else(|| EvalError::new(line, "data text size overflow"))?;
    let added_text = added
        .text_bytes
        .checked_add(added_key_bytes)
        .ok_or_else(|| EvalError::new(line, "data text size overflow"))?;
    let text_bytes = current
        .text_bytes
        .checked_sub(removed_text)
        .and_then(|bytes| bytes.checked_add(added_text))
        .ok_or_else(|| EvalError::new(line, "data text size overflow"))?;
    if text_bytes > MAX_DATA_TEXT_BYTES {
        return Err(EvalError::new(line, "data text exceeds 1 MiB"));
    }
    Ok(DataFootprint { values, text_bytes })
}

fn checked_total(
    current: DataFootprint,
    added: DataFootprint,
    max_values: usize,
    max_text_bytes: usize,
    context: &'static str,
) -> Result<DataFootprint, &'static str> {
    let values = current
        .values
        .checked_add(added.values)
        .ok_or("runtime data value count overflow")?;
    let text_bytes = current
        .text_bytes
        .checked_add(added.text_bytes)
        .ok_or("runtime data text size overflow")?;
    if values > max_values {
        return Err(match context {
            "host payload" => "host payload exceeds 100,000 values",
            _ => "machine state exceeds 100,000 values",
        });
    }
    if text_bytes > max_text_bytes {
        return Err(match context {
            "host payload" => "host payload text exceeds 16 MiB",
            _ => "machine state text exceeds 16 MiB",
        });
    }
    Ok(DataFootprint { values, text_bytes })
}

#[cfg(test)]
mod tests {
    use super::*;
    use velin_compile::ProgramBuilder;
    use velin_syntax::{BinaryOp, Expr};

    #[test]
    fn runs_assignments_and_conditionals_to_completion() {
        // hp = 30; if hp < 20 { hp = 0 }  -> hp stays 30
        let mut b = ProgramBuilder::new();
        let hp = b.slot("hp");
        let thirty = b.expr(&Expr::Value(Value::Integer(30)), 1);
        b.push(Op::Set {
            slot: hp,
            value: thirty,
        });
        let cond = b.expr(
            &Expr::Binary {
                left: Box::new(Expr::Variable("hp".into())),
                op: BinaryOp::Less,
                right: Box::new(Expr::Value(Value::Integer(20))),
            },
            2,
        );
        let jump = b.push(Op::JumpIfFalse {
            condition: cond,
            target: u32::MAX,
        });
        let zero = b.expr(&Expr::Value(Value::Integer(0)), 3);
        b.push(Op::Set {
            slot: hp,
            value: zero,
        });
        let end = b.here();
        b.patch(
            jump,
            Op::JumpIfFalse {
                condition: cond,
                target: end,
            },
        );
        let mut machine = Machine::new(b.build()).unwrap();
        assert_eq!(machine.run().unwrap(), Yield::Finished);
        assert_eq!(machine.variable("hp"), Some(&Value::Integer(30)));
    }

    #[test]
    fn direct_assignment_and_integer_guard_ops_preserve_semantics() {
        let mut builder = ProgramBuilder::new();
        let source = builder.slot("source");
        let copy = builder.slot("copy");
        let set = builder.set_op(source, &Expr::Value(Value::Integer(7)), 1);
        builder.push(set);
        let copy_op = builder.set_op(copy, &Expr::Variable("source".into()), 2);
        builder.push(copy_op);
        let comparison = Expr::Binary {
            left: Box::new(Expr::Variable("copy".into())),
            op: BinaryOp::GreaterEqual,
            right: Box::new(Expr::Value(Value::Integer(7))),
        };
        let guard = builder.jump_if_false_op(&comparison, 3, 5);
        builder.push(guard);
        let set = builder.set_op(copy, &Expr::Value(Value::Integer(9)), 4);
        builder.push(set);

        let mut machine = Machine::new(builder.build()).unwrap();
        assert_eq!(machine.run().unwrap(), Yield::Finished);
        assert_eq!(machine.variable("copy"), Some(&Value::Integer(9)));
    }

    #[test]
    fn quickened_readonly_builtins_preserve_results_and_errors() {
        let mut builder = ProgramBuilder::new();
        builder.slot("bag");
        for (target, source) in [
            ("size", "len(bag)"),
            ("present", "contains(bag, \"map\")"),
            ("first", "get(bag, 0)"),
            ("fallback", "get(bag, 9, \"missing\")"),
        ] {
            let target = builder.slot(target);
            let expression = velin_parse::parse_expression(source, "quickened", 4, 1).unwrap();
            let operation = builder.set_op(target, &expression, 4);
            builder.push(operation);
        }
        let program = builder.build();

        let mut unassigned = Machine::new(program.clone()).unwrap();
        let error = unassigned.run().unwrap_err();
        assert_eq!(error.line, 4);
        assert!(error.message.contains("`bag` has not been assigned"));

        let mut wrong_type = Machine::new(program.clone()).unwrap();
        wrong_type.set_variable("bag", Value::Integer(1));
        assert_eq!(
            wrong_type.run().unwrap_err().message,
            "len expects a list, record or string"
        );

        let mut machine = Machine::new(program).unwrap();
        machine.set_variable(
            "bag",
            Value::List(Arc::new(vec![Value::String("map".into())])),
        );
        assert_eq!(machine.run().unwrap(), Yield::Finished);
        assert_eq!(machine.variable("size"), Some(&Value::Integer(1)));
        assert_eq!(machine.variable("present"), Some(&Value::Boolean(true)));
        assert_eq!(
            machine.variable("first"),
            Some(&Value::String("map".into()))
        );
        assert_eq!(
            machine.variable("fallback"),
            Some(&Value::String("missing".into()))
        );
    }

    #[test]
    fn specialized_integer_guard_keeps_type_and_assignment_errors() {
        let mut builder = ProgramBuilder::new();
        builder.slot("source");
        let comparison = Expr::Binary {
            left: Box::new(Expr::Variable("source".into())),
            op: BinaryOp::Less,
            right: Box::new(Expr::Value(Value::Integer(7))),
        };
        let guard = builder.jump_if_false_op(&comparison, 8, 1);
        builder.push(guard);
        let program = builder.build();

        let mut unassigned = Machine::new(program.clone()).unwrap();
        let error = unassigned.run().unwrap_err();
        assert_eq!(error.line, 8);
        assert!(error.message.contains("has not been assigned"));

        let mut wrong_type = Machine::new(program).unwrap();
        wrong_type
            .try_set_variable("source", Value::Boolean(true))
            .unwrap();
        let error = wrong_type.run().unwrap_err();
        assert_eq!(error.line, 8);
        assert_eq!(
            error.message,
            "comparison cannot combine boolean and integer"
        );
    }

    #[test]
    fn yields_host_effects_and_binds_resume_values() {
        // ask(host 1); set choice from resume; done
        let mut b = ProgramBuilder::new();
        let choice = b.slot("choice");
        let prompt = b.expr(&Expr::Value(Value::String("pick".into())), 1);
        b.push(Op::host(1, vec![prompt], Some(choice), 1));
        let mut machine = Machine::new(b.build()).unwrap();
        let effect = machine.run().unwrap();
        assert_eq!(
            effect,
            Yield::Host {
                host_id: 1,
                values: vec![Value::String("pick".into())]
            }
        );
        assert_eq!(
            machine.resume(Some(Value::Integer(2))).unwrap(),
            Yield::Finished
        );
        assert_eq!(machine.variable("choice"), Some(&Value::Integer(2)));
    }

    #[test]
    fn infinite_loops_are_bounded() {
        // loop: jump loop
        let mut b = ProgramBuilder::new();
        b.push(Op::Jump(0));
        let mut machine = Machine::new(b.build()).unwrap();
        let error = machine.run().unwrap_err();
        assert!(error.message.contains("infinite loop"));
    }

    #[test]
    fn cloning_a_machine_rolls_back_rng_with_the_frame() {
        // Draw and yield once, clone the yielded machine as a checkpoint, then
        // draw again. Resuming the checkpoint must reproduce the second draw.
        let mut b = ProgramBuilder::new();
        let first = b.slot("first");
        let second = b.slot("second");
        let first_roll = velin_parse::parse_expression("random(1, 1000)", "t", 1, 1).unwrap();
        let first_chunk = b.expr(&first_roll, 1);
        b.push(Op::Set {
            slot: first,
            value: first_chunk,
        });
        let show_first = b.expr(&Expr::Variable("first".into()), 2);
        b.push(Op::host(1, vec![show_first], None, 2));
        let second_roll = velin_parse::parse_expression("random(1, 1000)", "t", 3, 1).unwrap();
        let second_chunk = b.expr(&second_roll, 3);
        b.push(Op::Set {
            slot: second,
            value: second_chunk,
        });
        let show_second = b.expr(&Expr::Variable("second".into()), 4);
        b.push(Op::host(2, vec![show_second], None, 4));

        let mut machine = Machine::with_seed(b.build(), 42).unwrap();
        assert!(matches!(
            machine.run().unwrap(),
            Yield::Host { host_id: 1, .. }
        ));
        let checkpoint = machine.clone();

        let expected = machine.resume(None).unwrap();
        let expected_state = machine.rng_state();
        let mut restored = checkpoint;
        let replayed = restored.resume(None).unwrap();
        assert_eq!(replayed, expected);
        assert_eq!(restored.rng_state(), expected_state);
        assert_eq!(restored.variable("second"), machine.variable("second"));
    }

    #[test]
    fn machine_snapshots_keep_independent_frames_after_writes() {
        let mut builder = ProgramBuilder::new();
        builder.slot("value");
        let mut machine = Machine::new(builder.build()).unwrap();
        machine
            .try_set_variable("value", Value::String("before".into()))
            .unwrap();
        let snapshot = machine.clone();

        machine
            .try_set_variable("value", Value::String("after".into()))
            .unwrap();
        assert_eq!(
            snapshot.variable("value"),
            Some(&Value::String("before".into()))
        );
        assert_eq!(
            machine.variable("value"),
            Some(&Value::String("after".into()))
        );
    }

    #[test]
    fn the_same_seed_replays_and_different_seeds_diverge() {
        let mut b = ProgramBuilder::new();
        let roll = b.slot("roll");
        let expression = velin_parse::parse_expression("random(0, 1000000)", "t", 1, 1).unwrap();
        let chunk = b.expr(&expression, 1);
        b.push(Op::Set {
            slot: roll,
            value: chunk,
        });
        let program = b.build();

        let mut first = Machine::with_seed(program.clone(), 5).unwrap();
        let mut replay = Machine::with_seed(program.clone(), 5).unwrap();
        let mut different = Machine::with_seed(program, 6).unwrap();
        first.run().unwrap();
        replay.run().unwrap();
        different.run().unwrap();
        assert_eq!(first.variable("roll"), replay.variable("roll"));
        assert_eq!(first.rng_state(), replay.rng_state());
        assert_ne!(first.rng_state(), different.rng_state());
    }

    #[test]
    fn non_boolean_conditions_are_rejected() {
        let mut b = ProgramBuilder::new();
        let condition = b.expr(&Expr::Value(Value::Integer(1)), 4);
        b.push(Op::JumpIfFalse {
            condition,
            target: 1,
        });
        let error = Machine::new(b.build()).unwrap().run().unwrap_err();
        assert_eq!(error.line, 4);
        assert!(error.message.contains("condition expects boolean"));
    }

    #[test]
    fn bound_host_effects_require_a_resume_value() {
        let mut b = ProgramBuilder::new();
        let choice = b.slot("choice");
        b.push(Op::host(1, Vec::new(), Some(choice), 7));
        let mut machine = Machine::new(b.build()).unwrap();
        machine.run().unwrap();

        let error = machine.resume(None).unwrap_err();
        assert_eq!(error.line, 7);
        assert!(error.message.contains("returned no value"));
        assert!(machine.variable("choice").is_none());

        let oversized = Value::String("x".repeat(velin_syntax::MAX_DATA_TEXT_BYTES + 1).into());
        let error = machine.resume(Some(oversized)).unwrap_err();
        assert_eq!(error.line, 7);
        assert!(error.message.contains("data text exceeds"));
        assert!(machine.variable("choice").is_none());

        assert_eq!(
            machine.resume(Some(Value::Integer(2))).unwrap(),
            Yield::Finished
        );
        assert_eq!(machine.variable("choice"), Some(&Value::Integer(2)));
    }

    #[test]
    fn run_cannot_skip_a_pending_host_effect() {
        let mut b = ProgramBuilder::new();
        b.push(Op::host(1, Vec::new(), None, 3));
        let mut machine = Machine::new(b.build()).unwrap();
        machine.run().unwrap();
        assert!(machine.run().unwrap_err().message.contains("call `resume`"));
        assert_eq!(machine.resume(None).unwrap(), Yield::Finished);
        assert!(
            machine
                .resume(None)
                .unwrap_err()
                .message
                .contains("no host effect")
        );
    }

    #[test]
    fn external_values_must_fit_the_data_budget() {
        let mut b = ProgramBuilder::new();
        b.slot("seed");
        let mut machine = Machine::new(b.build()).unwrap();
        let oversized = Value::String("x".repeat(velin_syntax::MAX_DATA_TEXT_BYTES + 1).into());
        assert!(!machine.set_variable("seed", oversized));
        assert!(machine.variable("seed").is_none());

        assert_eq!(
            machine
                .try_set_variable("missing", Value::Integer(1))
                .unwrap_err(),
            SetVariableError::UnknownVariable("missing".into())
        );
    }

    #[test]
    fn ownership_update_rejects_growth_without_changing_the_slot() {
        let mut builder = ProgramBuilder::new();
        let items = builder.slot("items");
        let value = builder.expr(&Expr::Value(Value::Integer(1)), 4);
        builder.push(Op::Update {
            slot: items,
            operation: UpdateOp::Push { value },
            line: 4,
            column: 1,
        });
        let mut machine = Machine::new(builder.build()).unwrap();
        let original = Value::List(Arc::new(vec![
            Value::Integer(0);
            velin_syntax::MAX_DATA_VALUES - 1
        ]));
        machine.try_set_variable("items", original.clone()).unwrap();
        let error = machine.run().unwrap_err();
        assert_eq!(
            error.message,
            "data exceeds 4096 values or 16 nesting levels"
        );
        assert_eq!(machine.variable("items"), Some(&original));
    }

    #[test]
    fn cached_collection_depth_is_recomputed_after_removing_the_deepest_child() {
        let mut builder = ProgramBuilder::new();
        let items = builder.slot("items");
        let outer = builder.slot("outer");
        let key = builder.expr(&Expr::Value(Value::Integer(0)), 1);
        builder.push(Op::update(items, UpdateOp::Remove { key }, 1, 1));
        let value = builder.expr(&Expr::Variable("items".into()), 2);
        builder.push(Op::update(outer, UpdateOp::Push { value }, 2, 1));

        let mut deep = Value::Integer(1);
        for _ in 0..15 {
            deep = Value::List(Arc::new(vec![deep]));
        }
        let mut machine = Machine::new(builder.build()).unwrap();
        machine
            .try_set_variable("items", Value::List(Arc::new(vec![deep])))
            .unwrap();
        machine
            .try_set_variable("outer", Value::List(Arc::new(Vec::new())))
            .unwrap();
        assert_eq!(machine.run().unwrap(), Yield::Finished);
        assert_eq!(
            machine.variable("outer"),
            Some(&Value::List(Arc::new(vec![Value::List(Arc::new(
                Vec::new()
            ))])))
        );
    }

    #[test]
    fn aggregate_machine_state_text_is_bounded_and_replacements_release_budget() {
        let mut builder = ProgramBuilder::new();
        let names: Vec<String> = (0..=16).map(|index| format!("value_{index}")).collect();
        for name in &names {
            builder.slot(name);
        }
        let mut machine = Machine::new(builder.build()).unwrap();
        let payload = Value::String("x".repeat(velin_syntax::MAX_DATA_TEXT_BYTES).into());

        for name in &names[..16] {
            machine.try_set_variable(name, payload.clone()).unwrap();
        }
        let error = machine
            .try_set_variable(&names[16], payload.clone())
            .unwrap_err();
        assert_eq!(
            error,
            SetVariableError::StateBudget("machine state text exceeds 16 MiB")
        );
        assert!(machine.variable(&names[16]).is_none());

        machine
            .try_set_variable(&names[0], Value::Integer(1))
            .unwrap();
        machine.try_set_variable(&names[16], payload).unwrap();
    }

    #[test]
    fn script_assignments_and_host_payloads_have_aggregate_budgets() {
        let payload = Value::String("x".repeat(velin_syntax::MAX_DATA_TEXT_BYTES).into());

        let mut assignments = ProgramBuilder::new();
        let chunk = assignments.expr(&Expr::Value(payload.clone()), 4);
        let slots: Vec<u32> = (0..=16)
            .map(|index| assignments.slot(&format!("value_{index}")))
            .collect();
        for slot in &slots {
            assignments.push(Op::Set {
                slot: *slot,
                value: chunk,
            });
        }
        let mut machine = Machine::new(assignments.build()).unwrap();
        let error = machine.run().unwrap_err();
        assert_eq!(error.line, 4);
        assert_eq!(error.message, "machine state text exceeds 16 MiB");
        assert!(machine.variable("value_16").is_none());

        let mut host = ProgramBuilder::new();
        let argument = host.expr(&Expr::Value(payload), 7);
        host.push(Op::host(1, vec![argument; 17], None, 7));
        let error = Machine::new(host.build()).unwrap().run().unwrap_err();
        assert_eq!(error.line, 7);
        assert_eq!(error.message, "host payload text exceeds 16 MiB");
    }

    #[test]
    fn rng_seed_and_unknown_variables_and_jump_off_end() {
        let mut plain = ProgramBuilder::new();
        plain.push(Op::Halt);
        let mut machine = Machine::new(plain.build()).unwrap();
        assert!(!machine.set_rng_seed(1));
        assert!(machine.rng_state().is_none());
        assert!(!machine.set_variable("missing", Value::Integer(1)));

        let mut random = ProgramBuilder::new();
        let roll = random.slot("roll");
        let expression = velin_parse::parse_expression("random(0, 10)", "t", 1, 1).unwrap();
        let chunk = random.expr(&expression, 1);
        random.push(Op::Set {
            slot: roll,
            value: chunk,
        });
        let mut machine = Machine::with_seed(random.build(), 1).unwrap();
        assert!(machine.set_rng_seed(99));
        assert_eq!(machine.rng_state(), Some(99));
        machine.set_variable(velin_compile::RNG_STATE_SLOT, Value::Boolean(true));
        assert!(machine.rng_state().is_none());

        let mut jump = ProgramBuilder::new();
        jump.push(Op::Jump(8));
        let error = Machine::new(jump.build()).unwrap_err();
        assert!(error.message.contains("past program end"));
    }
}
