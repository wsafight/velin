//! Pure, value-in/value-out module execution.
//!
//! A [`PureModule`] is a deliberately small embedding boundary around the
//! existing surface compiler and bytecode VM. Its only recognized host
//! commands are `return(value)` and `fail(message)`; every other host command
//! and all random built-ins are rejected while compiling the module.

use std::collections::BTreeMap;

use crate::{
    CompiledScript, Diagnostic, EvalError, HostSchema, HostSignature, Machine, Type, Value, Yield,
    compile,
};
use velin_bytecode::ExprOp;

/// A compiled, deterministic module with an explicit input contract.
#[derive(Debug, Clone)]
pub struct PureModule {
    source_name: String,
    script: CompiledScript,
    inputs: BTreeMap<String, Type>,
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
            Self::InvalidReturn(message) => formatter.write_str(message),
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

        Ok(Self {
            source_name,
            script,
            inputs,
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
        for name in values.keys() {
            if !self.inputs.contains_key(name) {
                return Err(PureModuleError::UnknownInput(name.clone()));
            }
        }
        for (name, expected) in &self.inputs {
            let value = values
                .get(name)
                .ok_or_else(|| PureModuleError::MissingInput(name.clone()))?;
            let found = Type::from(value);
            if !found.could_be(*expected) {
                return Err(PureModuleError::InputType {
                    name: name.clone(),
                    expected: *expected,
                    found,
                });
            }
        }

        let mut machine = self.new_machine()?;
        for (name, value) in values {
            machine.try_set_variable(&name, value).map_err(|error| {
                PureModuleError::Execution(EvalError::new(0, error.to_string()))
            })?;
        }

        match machine.run().map_err(PureModuleError::Execution)? {
            Yield::Finished => Err(PureModuleError::MissingReturn),
            Yield::Host { host_id, values } => {
                let name = self.script.host_name(host_id).unwrap_or("<unknown>");
                match name {
                    "return" => {
                        if values.len() != 1 {
                            return Err(PureModuleError::InvalidReturn(format!(
                                "return expects exactly one value, found {}",
                                values.len()
                            )));
                        }
                        values.into_iter().next().ok_or_else(|| {
                            PureModuleError::InvalidReturn(
                                "return expects exactly one value, found 0".into(),
                            )
                        })
                    }
                    "fail" => {
                        if values.len() != 1 {
                            return Err(PureModuleError::InvalidReturn(format!(
                                "fail expects exactly one value, found {}",
                                values.len()
                            )));
                        }
                        let Some(Value::String(message)) = values.into_iter().next() else {
                            return Err(PureModuleError::InvalidReturn(
                                "fail expects a string message".into(),
                            ));
                        };
                        Err(PureModuleError::ExplicitFailure(message.to_string()))
                    }
                    other => Err(PureModuleError::InvalidReturn(format!(
                        "unexpected host command `{other}` in pure module"
                    ))),
                }
            }
        }
    }

    fn new_machine(&self) -> Result<Machine, PureModuleError> {
        if let Some(frame) = self.script.initial_frame()
            && let Some(machine) = Machine::from_validated_with_seed_and_frame(
                self.script.validated_program(),
                crate::DEFAULT_RNG_SEED,
                frame,
            )
        {
            return Ok(machine);
        }

        // This fallback is only reachable if a caller mutates a public
        // `CompiledScript` field after embedding it. Keep it transactional and
        // report any resulting initialization failure as execution failure.
        let mut machine = Machine::from_validated_with_seed(
            self.script.validated_program(),
            crate::DEFAULT_RNG_SEED,
        );
        for (name, value) in &self.script.defaults {
            machine
                .try_set_variable(name, value.clone())
                .map_err(|error| {
                    PureModuleError::Execution(EvalError::new(0, error.to_string()))
                })?;
        }
        Ok(machine)
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
