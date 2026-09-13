//! Shared script instantiation and bounded host-effect driving.

use crate::{
    CompiledScript, DEFAULT_RNG_SEED, EvalError, HostSchema, Machine, ProgramValidationError,
    SetVariableError, Type, Value, Yield,
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
    HostEffectsExceeded {
        limit: usize,
    },
    HostContract(String),
}

impl std::fmt::Display for ScriptRunError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Program(error) => write!(formatter, "invalid bytecode: {error}"),
            Self::InitialValue { name, source } => {
                write!(formatter, "cannot initialize `{name}`: {source}")
            }
            Self::Evaluation(error) => write!(
                formatter,
                "runtime error at line {}: {}",
                error.line, error.message
            ),
            Self::HostEffectsExceeded { limit } => write!(
                formatter,
                "execution budget exceeded: too many host effects (limit {limit})"
            ),
            Self::HostContract(message) => write!(formatter, "host contract error: {message}"),
        }
    }
}

impl std::error::Error for ScriptRunError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Program(error) => Some(error),
            Self::InitialValue { source, .. } => Some(source),
            Self::Evaluation(error) => Some(error),
            Self::HostEffectsExceeded { .. } | Self::HostContract(_) => None,
        }
    }
}

/// A compiled script instance with shared initialization and execution limits.
pub struct ScriptRunner<'a> {
    script: &'a CompiledScript,
    schema: Option<&'a HostSchema>,
    machine: Machine,
    limits: ExecutionLimits,
    host_effects: usize,
    pending_host: Option<String>,
    pending_failure: Option<PendingFailure>,
}

#[derive(Debug, Clone)]
enum PendingFailure {
    HostEffectsExceeded { limit: usize },
    HostContract(String),
}

impl PendingFailure {
    fn error(&self) -> ScriptRunError {
        match self {
            Self::HostEffectsExceeded { limit } => {
                ScriptRunError::HostEffectsExceeded { limit: *limit }
            }
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
        let validated = script.validated_program().refers_to(&script.program);
        let prepared = validated
            .then(|| script.initial_frame())
            .flatten()
            .and_then(|frame| {
                Machine::from_validated_with_seed_and_frame(script.validated_program(), seed, frame)
            });
        let used_prepared_frame = prepared.is_some();
        let mut machine = if let Some(machine) = prepared {
            machine
        } else if validated {
            Machine::from_validated_with_seed(script.validated_program(), seed)
        } else {
            Machine::with_seed(script.program.clone(), seed).map_err(ScriptRunError::Program)?
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
        let outcome = self.machine.run().map_err(ScriptRunError::Evaluation)?;
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
        let count = self
            .machine
            .run_effect_batch_reusable(limit.min(remaining))
            .map_err(ScriptRunError::Evaluation)?;
        let raw = self.machine.effect_batch().to_vec();
        let mut events = Vec::with_capacity(count);
        for effect in raw {
            let Some(name) = self.script.host_name(effect.host_id) else {
                let failure = PendingFailure::HostContract(format!(
                    "bytecode yielded unknown host id {}",
                    effect.host_id
                ));
                let error = failure.error();
                self.pending_failure = Some(failure);
                return Err(error);
            };
            if let Err(message) = self.validate_call(name, &effect.values) {
                let failure = PendingFailure::HostContract(message);
                let error = failure.error();
                self.pending_failure = Some(failure);
                return Err(error);
            }
            events.push(HostEvent {
                name: name.to_owned(),
                values: effect.values,
            });
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
        let outcome = self
            .machine
            .resume(value)
            .map_err(ScriptRunError::Evaluation)?;
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

#[cfg(test)]
#[path = "runtime_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "runtime_coverage_tests.rs"]
mod coverage_tests;
