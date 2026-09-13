//! Incremental execution of independently compiled Velin fragments.
//!
//! A [`ReplSession`] keeps values by name, while every input is compiled into
//! a fresh [`CompiledScript`]. This preserves program validation boundaries and
//! makes a failed fragment unable to partially update session state.

use crate::{
    DEFAULT_RNG_SEED, Diagnostic, ExecutionLimits, ScriptRunError, ScriptRunner, ScriptYield,
    Value, compile, parse_program,
};
use std::collections::BTreeMap;
use std::fmt;
use velin_lang::Stmt;

const REPL_FILE: &str = "<repl>";

/// Why a REPL fragment was rejected or failed.
#[derive(Debug)]
pub enum ReplError {
    /// Parsing or lowering failed before a runnable script existed.
    Compile(Diagnostic),
    /// Static checks found a possible error using the session values as entry
    /// state. The session remains unchanged.
    Static(Vec<Diagnostic>),
    /// `default` declarations are deliberately kept out of fragments because
    /// defaults belong to a complete script's initial state.
    DefaultNotAllowed { line: usize },
    /// The temporary fragment runner failed. The session remains unchanged.
    Runtime(ScriptRunError),
    /// The host callback failed after an effect was handed to it. Host effects
    /// are external and cannot be rolled back; session values still are.
    Host(String),
}

impl fmt::Display for ReplError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Compile(error) => error.fmt(formatter),
            Self::Static(errors) => {
                if let Some(error) = errors.first() {
                    write!(formatter, "{error}")
                } else {
                    formatter.write_str("REPL static check failed")
                }
            }
            Self::DefaultNotAllowed { line } => {
                write!(
                    formatter,
                    "<repl>:{line}: default declarations are not supported"
                )
            }
            Self::Runtime(error) => error.fmt(formatter),
            Self::Host(message) => write!(formatter, "REPL host error: {message}"),
        }
    }
}

impl std::error::Error for ReplError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Compile(error) => Some(error),
            Self::Runtime(error) => Some(error),
            Self::Static(_) | Self::DefaultNotAllowed { .. } | Self::Host(_) => None,
        }
    }
}

/// State shared by a sequence of independently compiled input fragments.
#[derive(Debug, Clone, Default)]
pub struct ReplSession {
    values: BTreeMap<String, Value>,
    rng_state: Option<i64>,
    history: Vec<String>,
}

impl ReplSession {
    /// Creates an empty session using the standard deterministic RNG seed.
    #[must_use]
    pub fn new() -> Self {
        Self::with_seed(DEFAULT_RNG_SEED)
    }

    /// Creates an empty session with an explicit deterministic RNG seed.
    #[must_use]
    pub fn with_seed(seed: i64) -> Self {
        Self {
            values: BTreeMap::new(),
            rng_state: Some(seed),
            history: Vec::new(),
        }
    }

    /// Executes a fragment that must not yield a host effect.
    ///
    /// A fragment that performs a host command returns [`ReplError::Host`]
    /// before its session state is committed. Use [`Self::execute_with_host`]
    /// when host effects are expected.
    ///
    /// # Errors
    /// Returns a compile, static-check, runtime, or host-effect error. Failed
    /// fragments do not modify session values, RNG state, or history.
    pub fn execute(&mut self, source: &str) -> Result<(), ReplError> {
        self.execute_inner(source, |_name, _values| {
            Err("host effect requires execute_with_host".to_owned())
        })
    }

    /// Executes a fragment and handles each yielded host effect.
    ///
    /// The callback receives the command name and evaluated argument values,
    /// then returns the optional value supplied to a bound host command. Host
    /// effects already performed by the callback are intentionally not
    /// transactional; Velin session values and RNG state commit only after the
    /// fragment reaches `Finished`.
    ///
    /// # Errors
    /// Returns a compile, static-check, runtime, or callback error. A callback
    /// may already have performed an external effect when it returns an error.
    pub fn execute_with_host<F, E>(&mut self, source: &str, mut host: F) -> Result<(), ReplError>
    where
        F: FnMut(&str, &[Value]) -> Result<Option<Value>, E>,
        E: fmt::Display,
    {
        self.execute_inner(source, |name, values| {
            host(name, values).map_err(|error| error.to_string())
        })
    }

    fn execute_inner<F>(&mut self, source: &str, mut host: F) -> Result<(), ReplError>
    where
        F: FnMut(&str, &[Value]) -> Result<Option<Value>, String>,
    {
        let statements = parse_program(source)
            .map_err(|error| ReplError::Compile(error.into_diagnostic(REPL_FILE)))?;
        if let Some(line) = first_default(&statements) {
            return Err(ReplError::DefaultNotAllowed { line });
        }

        let script = compile(REPL_FILE, source).map_err(ReplError::Compile)?;
        let diagnostics = script.check_with_values(REPL_FILE, &self.values);
        if diagnostics.iter().any(Diagnostic::is_error) {
            return Err(ReplError::Static(diagnostics));
        }

        let seed = self.rng_state.unwrap_or(DEFAULT_RNG_SEED);
        let mut runner = ScriptRunner::configured(&script, seed, ExecutionLimits::default(), None)
            .map_err(ReplError::Runtime)?;
        for (name, value) in &self.values {
            if script.program.slots.get(name).is_some() {
                runner
                    .try_set_variable(name, value.clone())
                    .map_err(ReplError::Runtime)?;
            }
        }

        let mut outcome = runner.run().map_err(ReplError::Runtime)?;
        loop {
            match outcome {
                ScriptYield::Finished => break,
                ScriptYield::Host { name, values } => {
                    let reply = host(&name, &values).map_err(ReplError::Host)?;
                    outcome = runner.resume(reply).map_err(ReplError::Runtime)?;
                }
            }
        }

        for name in script.program.slots.names() {
            if name == velin_compile::RNG_STATE_SLOT {
                continue;
            }
            if let Some(value) = runner.machine().variable(name) {
                self.values.insert(name.clone(), value.clone());
            }
        }
        if let Some(state) = runner.machine().rng_state() {
            self.rng_state = Some(state);
        }
        self.history.push(source.to_owned());
        Ok(())
    }

    /// Returns the current value of a session variable.
    #[must_use]
    pub fn variable(&self, name: &str) -> Option<&Value> {
        self.values.get(name)
    }

    /// Returns all committed session variables in stable name order.
    #[must_use]
    pub const fn variables(&self) -> &BTreeMap<String, Value> {
        &self.values
    }

    /// Returns the deterministic RNG state after the most recent fragment
    /// that used randomness.
    #[must_use]
    pub const fn rng_state(&self) -> Option<i64> {
        self.rng_state
    }

    /// Returns successfully committed source fragments in input order.
    #[must_use]
    pub fn history(&self) -> &[String] {
        &self.history
    }
}

fn first_default(statements: &[Stmt]) -> Option<usize> {
    for statement in statements {
        match statement {
            Stmt::Default { line, .. } => return Some(*line),
            Stmt::Label { body, .. } | Stmt::While { body, .. } => {
                if let Some(line) = first_default(body) {
                    return Some(line);
                }
            }
            Stmt::If {
                branches,
                otherwise,
            } => {
                for branch in branches {
                    if let Some(line) = first_default(&branch.body) {
                        return Some(line);
                    }
                }
                if let Some(body) = otherwise
                    && let Some(line) = first_default(body)
                {
                    return Some(line);
                }
            }
            Stmt::Set { .. } | Stmt::Perform { .. } | Stmt::Jump { .. } => {}
        }
    }
    None
}

#[cfg(test)]
#[path = "repl_tests.rs"]
mod tests;
