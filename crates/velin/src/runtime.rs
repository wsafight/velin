//! Shared script instantiation and bounded host-effect driving.

use crate::{
    CompiledScript, DEFAULT_RNG_SEED, EvalError, EvalErrorKind, ExecutionPolicy, HostSchema,
    Machine, ProgramValidationError, SetVariableError, Type, Value, Yield,
};
use velin_bytecode::InitialFrame;

/// Default total host effects accepted during one script run.
pub const DEFAULT_MAX_HOST_EFFECTS: usize = 1_000;

/// Host-driving limits applied across every `run` / `resume` burst.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExecutionLimits {
    pub max_host_effects: usize,
}

impl Default for ExecutionLimits {
    fn default() -> Self {
        Self {
            max_host_effects: DEFAULT_MAX_HOST_EFFECTS,
        }
    }
}

/// A surface-language yield with its program-local id resolved to a name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScriptYield {
    Host { name: String, values: Vec<Value> },
    Finished,
}

mod error;
mod queue;
pub use queue::{HostEvent, HostEventQueue, HostEventQueueError, HostEventQueueLimits};

/// Failure while instantiating or driving a compiled script.
#[derive(Debug)]
pub enum ScriptRunError {
    Program(ProgramValidationError),
    InitialValue {
        name: String,
        source: SetVariableError,
    },
    Evaluation(EvalError),
    FuelExhausted {
        limit: u64,
        immediate: bool,
    },
    Cancelled,
    HostEffectsExceeded {
        limit: usize,
    },
    HostContract(String),
}

/// A compiled script instance with shared initialization and execution limits.
pub struct ScriptRunner<'a> {
    script: &'a CompiledScript,
    schema: Option<&'a HostSchema>,
    machine: Machine,
    policy: ExecutionPolicy,
    limits: ExecutionLimits,
    host_effects: usize,
    pending_host: Option<String>,
    pending_failure: Option<PendingFailure>,
}

#[derive(Debug, Clone)]
enum PendingFailure {
    HostEffectsExceeded { limit: usize },
    FuelExhausted { limit: u64, immediate: bool },
    HostContract(String),
}

impl PendingFailure {
    fn error(&self) -> ScriptRunError {
        match self {
            Self::HostEffectsExceeded { limit } => {
                ScriptRunError::HostEffectsExceeded { limit: *limit }
            }
            Self::FuelExhausted { limit, immediate } => ScriptRunError::FuelExhausted {
                limit: *limit,
                immediate: *immediate,
            },
            Self::HostContract(message) => ScriptRunError::HostContract(message.clone()),
        }
    }
}

impl<'a> ScriptRunner<'a> {
    /// Instantiates `script` with its defaults and the standard limits.
    ///
    /// # Errors
    /// Returns an error if bytecode validation or default initialization fails.
    pub fn new(script: &'a CompiledScript) -> Result<Self, ScriptRunError> {
        Self::configured(script, DEFAULT_RNG_SEED, ExecutionLimits::default(), None)
    }

    /// Instantiates a script with an explicit seed, limits, and optional host
    /// contract used for runtime argument and reply validation.
    ///
    /// # Errors
    /// Returns an error if bytecode validation or default initialization fails.
    pub fn configured(
        script: &'a CompiledScript,
        seed: i64,
        limits: ExecutionLimits,
        schema: Option<&'a HostSchema>,
    ) -> Result<Self, ScriptRunError> {
        let policy = ExecutionPolicy::default().with_max_host_effects(limits.max_host_effects);
        Self::configured_with_policy(script, seed, policy, schema)
    }

    /// Instantiates a script with an explicit VM execution policy and optional
    /// host contract used for runtime argument and reply validation.
    ///
    /// # Errors
    /// Returns an error if bytecode validation or policy-constrained default
    /// initialization fails.
    pub fn configured_with_policy(
        script: &'a CompiledScript,
        seed: i64,
        policy: ExecutionPolicy,
        schema: Option<&'a HostSchema>,
    ) -> Result<Self, ScriptRunError> {
        let limits = ExecutionLimits {
            max_host_effects: policy.max_host_effects,
        };
        let validated = script.validated_program().refers_to(&script.program);
        let prepared = validated
            .then(|| script.initial_frame())
            .flatten()
            .and_then(|frame| {
                Machine::from_validated_with_seed_and_frame_and_policy(
                    script.validated_program(),
                    seed,
                    frame,
                    policy.clone(),
                )
            });
        let used_prepared_frame = prepared.is_some();
        let mut machine = if let Some(machine) = prepared {
            machine
        } else if validated {
            Machine::from_validated_with_seed_and_policy(
                script.validated_program(),
                seed,
                policy.clone(),
            )
        } else {
            Machine::with_seed_and_policy(script.program.clone(), seed, policy.clone())
                .map_err(ScriptRunError::Program)?
        };
        if !used_prepared_frame {
            for (name, value) in &script.defaults {
                machine
                    .try_set_variable(name, value.clone())
                    .map_err(|source| ScriptRunError::InitialValue {
                        name: name.clone(),
                        source,
                    })?;
            }
        }
        Ok(Self {
            script,
            schema,
            machine,
            limits,
            policy,
            host_effects: 0,
            pending_host: None,
            pending_failure: None,
        })
    }

    #[must_use]
    pub const fn machine(&self) -> &Machine {
        &self.machine
    }

    /// Installs a value into a slot before running an independently compiled
    /// fragment. This is primarily useful for incremental frontends that keep
    /// their own session state.
    ///
    /// # Errors
    /// Returns an initialization error when `name` is unknown or the value
    /// would exceed the machine's aggregate data budget.
    pub fn try_set_variable(&mut self, name: &str, value: Value) -> Result<(), ScriptRunError> {
        self.machine
            .try_set_variable(name, value)
            .map_err(|source| ScriptRunError::InitialValue {
                name: name.to_owned(),
                source,
            })
    }

    #[must_use]
    pub const fn host_effects(&self) -> usize {
        self.host_effects
    }

    /// Returns the execution policy used by this runner.
    #[must_use]
    pub const fn policy(&self) -> &ExecutionPolicy {
        &self.policy
    }

    /// Returns cumulative VM fuel consumed since construction or restart.
    #[must_use]
    pub const fn fuel_used(&self) -> u64 {
        self.machine.fuel_used()
    }

    /// Requests cooperative cancellation at the next VM fuel checkpoint.
    pub fn cancel(&mut self) {
        self.machine.cancel();
    }

    /// Clears a previous cooperative cancellation request.
    pub fn clear_cancellation(&mut self) {
        self.machine.clear_cancellation();
    }

    /// Restarts this runner from the script's current defaults while reusing
    /// the machine's frame and register-workspace allocations.
    ///
    /// Defaults are rebuilt when the public map was changed after compilation;
    /// the machine is only modified after that frame and its budget pass.
    ///
    /// # Errors
    /// Returns an initialization or machine-state budget error without
    /// changing the current runner state.
    pub fn restart(&mut self, seed: i64) -> Result<(), ScriptRunError> {
        let frame = if let Some(frame) = self.script.initial_frame() {
            frame.clone()
        } else {
            InitialFrame::from_named_values(
                &self.script.program.slots,
                self.script
                    .defaults
                    .iter()
                    .map(|(name, value)| (name.as_str(), value)),
            )
            .map_err(|error| ScriptRunError::InitialValue {
                name: error.name,
                source: SetVariableError::InvalidValue(error.message),
            })?
        };
        self.machine
            .restart(&frame, seed)
            .map_err(|message| ScriptRunError::InitialValue {
                name: "<frame>".to_owned(),
                source: SetVariableError::StateBudget(message),
            })?;
        self.host_effects = 0;
        self.pending_host = None;
        self.pending_failure = None;
        Ok(())
    }

    /// Runs until the next host effect or completion.
    ///
    /// # Errors
    /// Returns evaluation, host-contract, or cumulative effect-budget errors.
    pub fn run(&mut self) -> Result<ScriptYield, ScriptRunError> {
        if let Some(failure) = &self.pending_failure {
            return Err(failure.error());
        }
        let outcome = match self.machine.run() {
            Ok(outcome) => outcome,
            Err(error) => return Err(self.map_machine_error(error)),
        };
        self.resolve(outcome)
    }

    /// Runs side-effect-only host commands in order and returns a batch of
    /// named events. A bound host command is left as the next VM barrier.
    ///
    /// The VM's reusable buffer is drained into the returned vector, so the
    /// next batch reuses its allocation. A configured host schema is checked
    /// for every event before it is returned to the host.
    ///
    /// # Errors
    /// Returns an error when a pending failure exists, the host-effect budget
    /// is exhausted, VM evaluation fails, or an event violates the host
    /// schema or payload limits.
    pub fn run_effect_batch(&mut self, limit: usize) -> Result<Vec<HostEvent>, ScriptRunError> {
        if let Some(failure) = &self.pending_failure {
            return Err(failure.error());
        }
        let remaining = self
            .limits
            .max_host_effects
            .checked_sub(self.host_effects)
            .ok_or(ScriptRunError::HostEffectsExceeded {
                limit: self.limits.max_host_effects,
            })?;
        if remaining == 0 {
            return Err(ScriptRunError::HostEffectsExceeded {
                limit: self.limits.max_host_effects,
            });
        }
        let count = match self.machine.run_effect_batch_reusable(limit.min(remaining)) {
            Ok(count) => count,
            Err(error) => return Err(self.map_machine_error(error)),
        };
        let raw = self.machine.effect_batch().to_vec();
        let mut events = Vec::with_capacity(count);
        let mut failure = None;
        for effect in raw {
            let Some(name) = self.script.host_name(effect.host_id) else {
                failure = Some(PendingFailure::HostContract(format!(
                    "bytecode yielded unknown host id {}",
                    effect.host_id
                )));
                break;
            };
            if let Err(message) = self.validate_call(name, &effect.values) {
                failure = Some(PendingFailure::HostContract(message));
                break;
            }
            events.push(HostEvent {
                name: name.to_owned(),
                values: effect.values,
            });
        }
        if let Some(failure) = failure {
            let error = failure.error();
            self.pending_failure = Some(failure);
            self.host_effects += events.len();
            let mut discarded = Vec::new();
            self.machine.drain_effect_batch(&mut discarded);
            return if events.is_empty() {
                Err(error)
            } else {
                Ok(events)
            };
        }
        self.host_effects += count;
        let mut discarded = Vec::new();
        self.machine.drain_effect_batch(&mut discarded);
        Ok(events)
    }

    /// Supplies the pending host reply and continues execution.
    ///
    /// A provided reply is checked against the pending command's schema before
    /// the VM consumes it, so the caller can retry after a contract error.
    ///
    /// # Errors
    /// Returns evaluation, host-contract, or cumulative effect-budget errors.
    pub fn resume(&mut self, value: Option<Value>) -> Result<ScriptYield, ScriptRunError> {
        if let Some(failure) = &self.pending_failure {
            return Err(failure.error());
        }
        self.validate_reply(value.as_ref())?;
        let outcome = match self.machine.resume(value) {
            Ok(outcome) => outcome,
            Err(error) => return Err(self.map_machine_error(error)),
        };
        self.pending_host = None;
        self.resolve(outcome)
    }

    fn resolve(&mut self, outcome: Yield) -> Result<ScriptYield, ScriptRunError> {
        match outcome {
            Yield::Finished => Ok(ScriptYield::Finished),
            Yield::Host { host_id, values } => {
                let Some(name) = self.script.host_name(host_id).map(str::to_owned) else {
                    return self.fail_pending(PendingFailure::HostContract(format!(
                        "bytecode yielded unknown host id {host_id}"
                    )));
                };
                if let Err(message) = self.validate_call(&name, &values) {
                    return self.fail_pending(PendingFailure::HostContract(message));
                }
                if self.host_effects >= self.limits.max_host_effects {
                    return self.fail_pending(PendingFailure::HostEffectsExceeded {
                        limit: self.limits.max_host_effects,
                    });
                }
                self.host_effects += 1;
                self.pending_host = Some(name.clone());
                Ok(ScriptYield::Host { name, values })
            }
        }
    }

    fn fail_pending(&mut self, failure: PendingFailure) -> Result<ScriptYield, ScriptRunError> {
        let error = failure.error();
        self.pending_failure = Some(failure);
        Err(error)
    }

    fn map_machine_error(&mut self, error: EvalError) -> ScriptRunError {
        let mapped = map_eval_error(error);
        match mapped {
            ScriptRunError::HostEffectsExceeded { limit } => {
                self.pending_host = None;
                self.pending_failure = Some(PendingFailure::HostEffectsExceeded { limit });
                ScriptRunError::HostEffectsExceeded { limit }
            }
            ScriptRunError::FuelExhausted { limit, immediate } => {
                self.pending_host = None;
                self.pending_failure = Some(PendingFailure::FuelExhausted { limit, immediate });
                ScriptRunError::FuelExhausted { limit, immediate }
            }
            other => other,
        }
    }

    fn validate_call(&self, name: &str, values: &[Value]) -> Result<(), String> {
        let Some(schema) = self.schema else {
            return Ok(());
        };
        let Some(signature) = schema.get(name) else {
            return if schema.allows_unknown() {
                Ok(())
            } else {
                Err(format!("command `{name}` is not declared"))
            };
        };
        if !signature.accepts(values.len()) {
            return Err(format!(
                "command `{name}` does not accept {} argument(s)",
                values.len()
            ));
        }
        for (index, value) in values.iter().enumerate() {
            let Some(expected) = signature.argument(index) else {
                continue;
            };
            let actual = Type::from(value);
            if !actual.could_be(expected) {
                return Err(format!(
                    "command `{name}` argument {} expects {}, found {}",
                    index + 1,
                    expected.name(),
                    actual.name()
                ));
            }
        }
        Ok(())
    }

    fn validate_reply(&self, value: Option<&Value>) -> Result<(), ScriptRunError> {
        let (Some(schema), Some(name), Some(value)) =
            (self.schema, self.pending_host.as_deref(), value)
        else {
            return Ok(());
        };
        let Some(signature) = schema.get(name) else {
            return Ok(());
        };
        let Some(expected) = signature.returns() else {
            return Err(ScriptRunError::HostContract(format!(
                "command `{name}` does not return a value"
            )));
        };
        let actual = Type::from(value);
        if actual.could_be(expected) {
            Ok(())
        } else {
            Err(ScriptRunError::HostContract(format!(
                "command `{name}` returns {}, found {}",
                expected.name(),
                actual.name()
            )))
        }
    }
}

fn map_eval_error(error: EvalError) -> ScriptRunError {
    match error.kind() {
        EvalErrorKind::FuelExhausted => ScriptRunError::FuelExhausted {
            limit: error.limit().unwrap_or_default(),
            immediate: error.message.contains("immediate"),
        },
        EvalErrorKind::Cancelled => ScriptRunError::Cancelled,
        EvalErrorKind::HostEffectsExceeded => ScriptRunError::HostEffectsExceeded {
            limit: usize::try_from(error.limit().unwrap_or_default()).unwrap_or(usize::MAX),
        },
        EvalErrorKind::Runtime => ScriptRunError::Evaluation(error),
    }
}

#[cfg(test)]
#[path = "runtime_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "runtime_coverage_tests.rs"]
mod coverage_tests;
