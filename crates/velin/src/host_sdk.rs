//! Schema-backed synchronous and asynchronous host drivers.

use crate::{
    CompiledScript, Diagnostic, ExecutionPolicy, HostCommand, HostSchema, Machine, ScriptRunError,
    ScriptRunner, ScriptYield, Value,
};
use std::collections::BTreeMap;
use std::fmt;
use std::future::Future;
use std::pin::Pin;

type SyncHandler<E> = Box<dyn FnMut(&[Value]) -> Result<Option<Value>, E> + 'static>;
type AsyncHandlerFuture<E> = Pin<Box<dyn Future<Output = Result<Option<Value>, E>> + 'static>>;
type AsyncHandler<E> = Box<dyn FnMut(Vec<Value>) -> AsyncHandlerFuture<E> + 'static>;

/// Failure while checking or driving a schema-backed host.
#[derive(Debug)]
pub enum HostDriveError<E> {
    Static(Vec<Diagnostic>),
    Runtime(ScriptRunError),
    MissingHandler { command: String },
    Handler { command: String, source: E },
}

impl<E: fmt::Display> fmt::Display for HostDriveError<E> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Static(diagnostics) => {
                write!(
                    formatter,
                    "script has {} static error(s)",
                    diagnostics.len()
                )
            }
            Self::Runtime(error) => error.fmt(formatter),
            Self::MissingHandler { command } => {
                write!(
                    formatter,
                    "host command `{command}` has no registered handler"
                )
            }
            Self::Handler { command, source } => {
                write!(formatter, "host command `{command}` failed: {source}")
            }
        }
    }
}

impl<E> std::error::Error for HostDriveError<E>
where
    E: std::error::Error + 'static,
{
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Runtime(error) => Some(error),
            Self::Handler { source, .. } => Some(source),
            Self::Static(_) | Self::MissingHandler { .. } => None,
        }
    }
}

/// A synchronous host whose declarations and handlers are registered together.
pub struct SyncHostDriver<E> {
    schema: HostSchema,
    handlers: BTreeMap<String, SyncHandler<E>>,
}

impl<E> Default for SyncHostDriver<E> {
    fn default() -> Self {
        Self::new()
    }
}

impl<E> SyncHostDriver<E> {
    #[must_use]
    pub fn new() -> Self {
        Self {
            schema: HostSchema::new(),
            handlers: BTreeMap::new(),
        }
    }

    /// Registers one declaration and its implementation as a single operation.
    #[must_use]
    pub fn command<F>(mut self, command: HostCommand, handler: F) -> Self
    where
        F: FnMut(&[Value]) -> Result<Option<Value>, E> + 'static,
    {
        let name = command.name().to_owned();
        self.schema.insert_command(command);
        self.handlers.insert(name, Box::new(handler));
        self
    }

    #[must_use]
    pub const fn schema(&self) -> &HostSchema {
        &self.schema
    }

    /// Checks and runs a script with the default execution policy.
    ///
    /// # Errors
    /// Returns static diagnostics, runtime contract failures, missing handlers,
    /// or the handler's own error.
    pub fn run(
        &mut self,
        file: &str,
        script: &CompiledScript,
    ) -> Result<Machine, HostDriveError<E>> {
        self.run_with_policy(file, script, 0, ExecutionPolicy::default())
    }

    /// Checks and runs a script with an explicit seed and execution policy.
    ///
    /// # Errors
    /// Returns static diagnostics, runtime contract failures, missing handlers,
    /// or the handler's own error.
    pub fn run_with_policy(
        &mut self,
        file: &str,
        script: &CompiledScript,
        seed: i64,
        policy: ExecutionPolicy,
    ) -> Result<Machine, HostDriveError<E>> {
        reject_static_errors(file, script, &self.schema)?;
        let schema = &self.schema;
        let handlers = &mut self.handlers;
        let mut runner = ScriptRunner::configured_with_policy(script, seed, policy, Some(schema))
            .map_err(HostDriveError::Runtime)?;
        let mut outcome = runner.run().map_err(HostDriveError::Runtime)?;
        loop {
            match outcome {
                ScriptYield::Finished => return Ok(runner.into_machine()),
                ScriptYield::Host { name, values } => {
                    let Some(handler) = handlers.get_mut(&name) else {
                        return Err(HostDriveError::MissingHandler { command: name });
                    };
                    let reply = handler(&values).map_err(|source| HostDriveError::Handler {
                        command: name,
                        source,
                    })?;
                    outcome = runner.resume(reply).map_err(HostDriveError::Runtime)?;
                }
            }
        }
    }
}

/// An asynchronous host driver with owned arguments at each await boundary.
pub struct AsyncHostDriver<E> {
    schema: HostSchema,
    handlers: BTreeMap<String, AsyncHandler<E>>,
}

impl<E> Default for AsyncHostDriver<E> {
    fn default() -> Self {
        Self::new()
    }
}

impl<E> AsyncHostDriver<E> {
    #[must_use]
    pub fn new() -> Self {
        Self {
            schema: HostSchema::new(),
            handlers: BTreeMap::new(),
        }
    }

    /// Registers one declaration and asynchronous implementation together.
    #[must_use]
    pub fn command<F, Fut>(mut self, command: HostCommand, mut handler: F) -> Self
    where
        F: FnMut(Vec<Value>) -> Fut + 'static,
        Fut: Future<Output = Result<Option<Value>, E>> + 'static,
    {
        let name = command.name().to_owned();
        self.schema.insert_command(command);
        self.handlers
            .insert(name, Box::new(move |values| Box::pin(handler(values))));
        self
    }

    #[must_use]
    pub const fn schema(&self) -> &HostSchema {
        &self.schema
    }

    /// Checks and runs a script with the default execution policy.
    ///
    /// # Errors
    /// Returns static diagnostics, runtime contract failures, missing handlers,
    /// or the asynchronous handler's own error.
    pub async fn run(
        &mut self,
        file: &str,
        script: &CompiledScript,
    ) -> Result<Machine, HostDriveError<E>> {
        self.run_with_policy(file, script, 0, ExecutionPolicy::default())
            .await
    }

    /// Checks and runs a script with an explicit seed and execution policy.
    ///
    /// # Errors
    /// Returns static diagnostics, runtime contract failures, missing handlers,
    /// or the asynchronous handler's own error.
    pub async fn run_with_policy(
        &mut self,
        file: &str,
        script: &CompiledScript,
        seed: i64,
        policy: ExecutionPolicy,
    ) -> Result<Machine, HostDriveError<E>> {
        reject_static_errors(file, script, &self.schema)?;
        let schema = &self.schema;
        let handlers = &mut self.handlers;
        let mut runner = ScriptRunner::configured_with_policy(script, seed, policy, Some(schema))
            .map_err(HostDriveError::Runtime)?;
        let mut outcome = runner.run().map_err(HostDriveError::Runtime)?;
        loop {
            match outcome {
                ScriptYield::Finished => return Ok(runner.into_machine()),
                ScriptYield::Host { name, values } => {
                    let Some(handler) = handlers.get_mut(&name) else {
                        return Err(HostDriveError::MissingHandler { command: name });
                    };
                    let reply =
                        handler(values)
                            .await
                            .map_err(|source| HostDriveError::Handler {
                                command: name,
                                source,
                            })?;
                    outcome = runner.resume(reply).map_err(HostDriveError::Runtime)?;
                }
            }
        }
    }
}

fn reject_static_errors<E>(
    file: &str,
    script: &CompiledScript,
    schema: &HostSchema,
) -> Result<(), HostDriveError<E>> {
    let errors: Vec<_> = script
        .check_with_host_schema(file, schema)
        .into_iter()
        .filter(Diagnostic::is_error)
        .collect();
    if errors.is_empty() {
        Ok(())
    } else {
        Err(HostDriveError::Static(errors))
    }
}

#[cfg(test)]
#[path = "host_sdk_tests.rs"]
mod tests;
