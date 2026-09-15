//! Pure, value-in/value-out module execution.
//!
//! A [`PureModule`] is a deliberately small embedding boundary around the
//! existing surface compiler and bytecode VM. Its only recognized host
//! commands are `return(value)` and `fail(message)`; every other host command
//! and all random built-ins are rejected while compiling the module.

use std::collections::BTreeMap;

use crate::{
    CompiledScript, Diagnostic, EvalError, FastYield, HostSchema, HostSignature, Machine,
    MachineInvoker, Type, Value, compile,
};
use velin_bytecode::ExprOp;

/// A compiled, deterministic module with an explicit input contract.
#[derive(Debug, Clone)]
pub struct PureModule {
    source_name: String,
    script: CompiledScript,
    inputs: BTreeMap<String, Type>,
    bindings: Box<[InputBinding]>,
    return_host_id: Option<u32>,
    fail_host_id: Option<u32>,
}

#[derive(Debug, Clone)]
struct InputBinding {
    name: String,
    slot: u32,
    expected: Type,
}

/// Failure while compiling or invoking a [`PureModule`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PureModuleError {
    /// Parsing or lowering failed before a runnable script was produced.
    Compile(Diagnostic),
    /// Static assignment, type, or host-contract checks failed.
    Check(Vec<Diagnostic>),
    /// The invocation omitted one of the declared input names.
    MissingInput(String),
    /// The invocation supplied a name absent from the module input contract.
    UnknownInput(String),
    /// A convenience invocation shape does not match the module input arity.
    InvalidInput(String),
    /// An invocation value does not satisfy its declared input type.
    InputType {
        name: String,
        expected: Type,
        found: Type,
    },
    /// Execution reached the end of the program without `return(value)`.
    MissingReturn,
    /// The reserved return/fail protocol was malformed at runtime.
    InvalidReturn(String),
    /// The module explicitly stopped with `fail(message)`.
    ExplicitFailure(String),
    /// The underlying VM rejected an expression or execution step.
    Execution(EvalError),
}

impl std::fmt::Display for PureModuleError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Compile(error) => write!(formatter, "compile error: {error}"),
            Self::Check(errors) => {
                write!(formatter, "pure module check failed")?;
                for error in errors {
                    write!(formatter, "\n{error}")?;
                }
                Ok(())
            }
            Self::MissingInput(name) => write!(formatter, "missing input `{name}`"),
            Self::UnknownInput(name) => write!(formatter, "unknown input `{name}`"),
            Self::InvalidInput(message) | Self::InvalidReturn(message) => {
                formatter.write_str(message)
            }
            Self::InputType {
                name,
                expected,
                found,
            } => write!(
                formatter,
                "input `{name}` expects {}, found {}",
                expected.name(),
                found.name()
            ),
            Self::MissingReturn => formatter.write_str("pure module finished without return"),
            Self::ExplicitFailure(message) => write!(formatter, "pure module failed: {message}"),
            Self::Execution(error) => write!(formatter, "execution error: {error}"),
        }
    }
}

impl std::error::Error for PureModuleError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Compile(error) => Some(error),
            Self::Execution(error) => Some(error),
            Self::Check(_)
            | Self::MissingInput(_)
            | Self::UnknownInput(_)
            | Self::InvalidInput(_)
            | Self::InputType { .. }
            | Self::MissingReturn
            | Self::InvalidReturn(_)
            | Self::ExplicitFailure(_) => None,
        }
    }
}

impl PureModule {
    /// Compiles a source module with the supplied input names and static types.
    ///
    /// The module may use ordinary Velin expressions and control flow. Its
    /// only permitted effects are `perform return(value)` and
    /// `perform fail(message)`.
    ///
    /// # Errors
    /// Returns [`PureModuleError::Compile`] for parser/lowering failures and
    /// [`PureModuleError::Check`] for static contract violations.
    pub fn compile(
        source_name: impl Into<String>,
        source: &str,
        inputs: BTreeMap<String, Type>,
    ) -> Result<Self, PureModuleError> {
        let source_name = source_name.into();
        let script = compile(&source_name, source).map_err(PureModuleError::Compile)?;

        // An input is legal only when the compiled program has a corresponding
        // named slot. This catches typos even when a declared input is unused.
        if let Some(name) = inputs
            .keys()
            .find(|name| script.program.slots.get(name.as_str()).is_none())
        {
            return Err(PureModuleError::UnknownInput(name.clone()));
        }

        let schema = pure_host_schema();
        let mut diagnostics =
            script.check_with_bindings_and_host_schema(&source_name, &inputs, &schema);
        diagnostics.extend(random_diagnostics(&source_name, &script));
        if !diagnostics.is_empty() {
            return Err(PureModuleError::Check(diagnostics));
        }

        let bindings = inputs
            .iter()
            .filter_map(|(name, expected)| {
                script.program.slots.get(name).map(|slot| InputBinding {
                    name: name.clone(),
                    slot,
                    expected: *expected,
                })
            })
            .collect();
        let return_host_id = script
            .hosts
            .iter()
            .position(|name| name == "return")
            .and_then(|id| u32::try_from(id).ok());
        let fail_host_id = script
            .hosts
            .iter()
            .position(|name| name == "fail")
            .and_then(|id| u32::try_from(id).ok());

        Ok(Self {
            source_name,
            script,
            inputs,
            bindings,
            return_host_id,
            fail_host_id,
        })
    }

    /// Returns the source name used for diagnostics.
    #[must_use]
    pub fn source_name(&self) -> &str {
        &self.source_name
    }

    /// Returns the declared input type contract.
    #[must_use]
    pub fn inputs(&self) -> &BTreeMap<String, Type> {
        &self.inputs
    }

    /// Returns the underlying compiled script for inspection and tooling.
    #[must_use]
    pub fn script(&self) -> &CompiledScript {
        &self.script
    }

    /// Invokes the module with one fresh VM state.
    ///
    /// Inputs are validated before any machine state is changed. A successful
    /// call returns the value supplied to `return`; `fail(message)` becomes an
    /// [`PureModuleError::ExplicitFailure`].
    ///
    /// # Errors
    /// Returns an input, protocol, static-execution, or explicit module error.
    pub fn invoke(&self, values: BTreeMap<String, Value>) -> Result<Value, PureModuleError> {
        let mut invoker = self.invoker()?;
        invoker.invoke(values)
    }

    /// Creates a reusable pure-module invocation session.
    ///
    /// The session owns one mutable machine and can be used by one concurrent
    /// caller. Each invocation restarts from the compiled initial frame.
    ///
    /// # Errors
    /// Returns an execution error if the compiled initial frame is invalid.
    pub fn invoker(&self) -> Result<PureModuleInvoker<'_>, PureModuleError> {
        let initial = self.script.initial_frame().ok_or_else(|| {
            PureModuleError::Execution(EvalError::new(0, "compiled initial frame is invalid"))
        })?;
        let machine = MachineInvoker::new(
            self.script.validated_program(),
            crate::DEFAULT_RNG_SEED,
            initial,
        )
        .ok_or_else(|| {
            PureModuleError::Execution(EvalError::new(0, "compiled initial frame is invalid"))
        })?;
        Ok(PureModuleInvoker {
            module: self,
            machine,
        })
    }

    fn validate_values(&self, values: &BTreeMap<String, Value>) -> Result<(), PureModuleError> {
        for name in values.keys() {
            if !self.inputs.contains_key(name) {
                return Err(PureModuleError::UnknownInput(name.clone()));
            }
        }
        for binding in &self.bindings {
            let value = values
                .get(&binding.name)
                .ok_or_else(|| PureModuleError::MissingInput(binding.name.clone()))?;
            let found = Type::from(value);
            if !found.could_be(binding.expected) {
                return Err(PureModuleError::InputType {
                    name: binding.name.clone(),
                    expected: binding.expected,
                    found,
                });
            }
        }
        Ok(())
    }

    fn finish_invocation(&self, machine: &mut Machine) -> Result<Value, PureModuleError> {
        let mut fast_hosts = [0; 2];
        let mut fast_host_count = 0;
        for host_id in [self.return_host_id, self.fail_host_id]
            .into_iter()
            .flatten()
        {
            if fast_host_count < fast_hosts.len() {
                fast_hosts[fast_host_count] = host_id;
                fast_host_count += 1;
            }
        }
        match machine
            .run_with_single_argument_hosts(&fast_hosts[..fast_host_count])
            .map_err(PureModuleError::Execution)?
        {
            FastYield::Finished => Err(PureModuleError::MissingReturn),
            FastYield::HostOne { host_id, value } => self.finish_single_host(host_id, value),
            FastYield::Host { host_id, values } => {
                if values.len() != 1 {
                    return Err(PureModuleError::InvalidReturn(format!(
                        "host command expects exactly one value, found {}",
                        values.len()
                    )));
                }
                let value = values.into_iter().next().ok_or_else(|| {
                    PureModuleError::InvalidReturn("host command returned no value".into())
                })?;
                self.finish_single_host(host_id, value)
            }
        }
    }

    fn finish_single_host(&self, host_id: u32, value: Value) -> Result<Value, PureModuleError> {
        if Some(host_id) == self.return_host_id {
            return Ok(value);
        }
        if Some(host_id) == self.fail_host_id {
            let Value::String(message) = value else {
                return Err(PureModuleError::InvalidReturn(
                    "fail expects a string message".into(),
                ));
            };
            return Err(PureModuleError::ExplicitFailure(message.to_string()));
        }
        let name = self.script.host_name(host_id).unwrap_or("<unknown>");
        Err(PureModuleError::InvalidReturn(format!(
            "unexpected host command `{name}` in pure module"
        )))
    }
}

/// A reusable pure-module invocation session.
#[derive(Debug)]
pub struct PureModuleInvoker<'a> {
    module: &'a PureModule,
    machine: MachineInvoker,
}

impl PureModuleInvoker<'_> {
    /// Invokes the module after resetting its machine to the initial frame.
    ///
    /// # Errors
    /// Returns an input, execution, protocol, or explicit module error.
    pub fn invoke(
        &mut self,
        mut values: BTreeMap<String, Value>,
    ) -> Result<Value, PureModuleError> {
        self.module.validate_values(&values)?;
        self.machine
            .restart()
            .map_err(|error| PureModuleError::Execution(EvalError::new(0, error)))?;
        for binding in &self.module.bindings {
            let Some(value) = values.remove(&binding.name) else {
                return Err(PureModuleError::MissingInput(binding.name.clone()));
            };
            self.machine
                .machine_mut()
                .try_set_slot(binding.slot, value)
                .map_err(|error| {
                    PureModuleError::Execution(EvalError::new(0, error.to_string()))
                })?;
        }
        self.module.finish_invocation(self.machine.machine_mut())
    }

    /// Invokes a module that declares exactly one input, without creating a
    /// map or looking up the input name at runtime.
    ///
    /// # Errors
    /// Returns an arity, type, execution, protocol, or explicit module error.
    pub fn invoke_one(&mut self, value: Value) -> Result<Value, PureModuleError> {
        let [binding] = self.module.bindings.as_ref() else {
            return Err(PureModuleError::InvalidInput(
                "invoke_one requires exactly one declared input".into(),
            ));
        };
        let found = Type::from(&value);
        if !found.could_be(binding.expected) {
            return Err(PureModuleError::InputType {
                name: binding.name.clone(),
                expected: binding.expected,
                found,
            });
        }
        self.machine
            .restart()
            .map_err(|error| PureModuleError::Execution(EvalError::new(0, error)))?;
        self.machine
            .machine_mut()
            .try_set_slot(binding.slot, value)
            .map_err(|error| PureModuleError::Execution(EvalError::new(0, error.to_string())))?;
        self.module.finish_invocation(self.machine.machine_mut())
    }

    /// Invokes the same module repeatedly while reusing the session buffers.
    ///
    /// # Errors
    /// Returns the first input, execution, protocol, or explicit module error.
    pub fn invoke_batch<I>(&mut self, inputs: I) -> Result<Vec<Value>, PureModuleError>
    where
        I: IntoIterator<Item = BTreeMap<String, Value>>,
    {
        inputs.into_iter().map(|input| self.invoke(input)).collect()
    }

    /// Returns the underlying reusable machine for advanced host integration.
    #[must_use]
    pub const fn machine(&self) -> &Machine {
        self.machine.machine()
    }
}

fn pure_host_schema() -> HostSchema {
    HostSchema::new()
        .command("return", HostSignature::exact(vec![Type::Unknown], None))
        .command("fail", HostSignature::exact(vec![Type::String], None))
}

fn random_diagnostics(file: &str, script: &CompiledScript) -> Vec<Diagnostic> {
    script
        .program
        .chunks
        .iter()
        .filter_map(|chunk| {
            let ops = script
                .program
                .expr_ops
                .get(chunk.ops.start as usize..chunk.ops.end as usize)?;
            ops.iter()
                .any(|op| matches!(op, ExprOp::Random { .. } | ExprOp::Chance { .. }))
                .then(|| {
                    Diagnostic::new(
                        file,
                        chunk.line as usize,
                        1,
                        "random and chance are not allowed in pure modules",
                    )
                })
        })
        .collect()
}
