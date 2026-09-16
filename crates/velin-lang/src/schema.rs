//! Host-command declarations shared by checking, runtime dispatch, and tools.

use std::collections::BTreeMap;
use std::fmt;
use velin_check::{HostSignature, Type};
use velin_syntax::Value;

/// One host command's complete declaration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostCommand {
    name: String,
    signature: HostSignature,
    description: Option<String>,
}

impl HostCommand {
    /// Creates a command declaration without documentation.
    #[must_use]
    pub fn new(name: impl Into<String>, signature: HostSignature) -> Self {
        Self {
            name: name.into(),
            signature,
            description: None,
        }
    }

    /// Adds editor-facing documentation to this command.
    #[must_use]
    pub fn description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    #[must_use]
    pub const fn signature(&self) -> &HostSignature {
        &self.signature
    }

    #[must_use]
    pub fn documentation(&self) -> Option<&str> {
        self.description.as_deref()
    }

    /// Formats the declaration for editor completion, hover, and signature help.
    #[must_use]
    pub fn signature_label(&self) -> String {
        let mut label = String::new();
        label.push_str(&self.name);
        label.push('(');
        for (index, argument) in self.signature.arguments().iter().enumerate() {
            if index != 0 {
                label.push_str(", ");
            }
            label.push_str(argument.name());
        }
        if let Some(variadic) = self.signature.variadic_type() {
            if !self.signature.arguments().is_empty() {
                label.push_str(", ");
            }
            label.push_str(variadic.name());
            label.push_str("...");
        }
        label.push(')');
        if let Some(returns) = self.signature.returns() {
            label.push_str(" -> ");
            label.push_str(returns.name());
        }
        label
    }
}

/// A host contract violation found before dispatch or resume.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostContractError {
    message: String,
}

impl HostContractError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for HostContractError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for HostContractError {}

/// Host-owned command declarations used by every integration layer.
#[derive(Debug, Clone, Default)]
pub struct HostSchema {
    commands: BTreeMap<String, HostCommand>,
    allow_unknown: bool,
}

impl HostSchema {
    /// Creates a strict schema that reports every undeclared command.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Controls whether undeclared commands remain valid opaque effects.
    #[must_use]
    pub const fn allow_unknown(mut self, allow: bool) -> Self {
        self.allow_unknown = allow;
        self
    }

    /// Adds a command contract without documentation.
    #[must_use]
    pub fn command(self, name: impl Into<String>, signature: HostSignature) -> Self {
        self.declare(HostCommand::new(name, signature))
    }

    /// Adds a complete command declaration.
    #[must_use]
    pub fn declare(mut self, command: HostCommand) -> Self {
        self.insert_command(command);
        self
    }

    /// Adds a command contract, returning the previous signature.
    pub fn insert(
        &mut self,
        name: impl Into<String>,
        signature: HostSignature,
    ) -> Option<HostSignature> {
        self.insert_command(HostCommand::new(name, signature))
            .map(|command| command.signature)
    }

    /// Adds a complete declaration, returning the previous declaration.
    pub fn insert_command(&mut self, command: HostCommand) -> Option<HostCommand> {
        self.commands.insert(command.name.clone(), command)
    }

    /// Returns a command's signature for compatibility with existing hosts.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&HostSignature> {
        self.command_info(name).map(HostCommand::signature)
    }

    /// Returns a command's full declaration.
    #[must_use]
    pub fn command_info(&self, name: &str) -> Option<&HostCommand> {
        self.commands.get(name)
    }

    /// Iterates declarations in stable command-name order.
    #[must_use]
    pub fn commands(&self) -> impl ExactSizeIterator<Item = &HostCommand> {
        self.commands.values()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.commands.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.commands.is_empty()
    }

    #[must_use]
    pub const fn allows_unknown(&self) -> bool {
        self.allow_unknown
    }

    /// Validates arguments before exposing an effect to host code.
    ///
    /// # Errors
    /// Returns a contract error for an undeclared strict-schema command, an
    /// invalid argument count, or a known argument-type mismatch.
    pub fn validate_call(
        &self,
        name: &str,
        values: &[Value],
    ) -> Result<Option<&HostCommand>, HostContractError> {
        let Some(command) = self.command_info(name) else {
            return if self.allow_unknown {
                Ok(None)
            } else {
                Err(HostContractError::new(format!(
                    "command `{name}` is not declared"
                )))
            };
        };
        let signature = command.signature();
        if !signature.accepts(values.len()) {
            return Err(HostContractError::new(format!(
                "command `{name}` does not accept {} argument(s)",
                values.len()
            )));
        }
        for (index, value) in values.iter().enumerate() {
            let Some(expected) = signature.argument(index) else {
                continue;
            };
            let actual = Type::from(value);
            if !actual.could_be(expected) {
                return Err(HostContractError::new(format!(
                    "command `{name}` argument {} expects {}, found {}",
                    index + 1,
                    expected.name(),
                    actual.name()
                )));
            }
        }
        Ok(Some(command))
    }

    /// Validates a supplied reply against a declared return type.
    ///
    /// # Errors
    /// Returns a contract error when a value is supplied to a command without
    /// a return type or when its known type differs from the declaration.
    pub fn validate_reply(
        &self,
        name: &str,
        value: Option<&Value>,
    ) -> Result<(), HostContractError> {
        let (Some(command), Some(value)) = (self.command_info(name), value) else {
            return Ok(());
        };
        let Some(expected) = command.signature().returns() else {
            return Err(HostContractError::new(format!(
                "command `{name}` does not return a value"
            )));
        };
        let actual = Type::from(value);
        if actual.could_be(expected) {
            Ok(())
        } else {
            Err(HostContractError::new(format!(
                "command `{name}` returns {}, found {}",
                expected.name(),
                actual.name()
            )))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn declarations_are_stable_and_validate_both_directions() {
        let schema = HostSchema::new().declare(
            HostCommand::new(
                "ask",
                HostSignature::exact(vec![Type::String], Some(Type::Integer)),
            )
            .description("Ask the current player."),
        );
        let command = schema.commands().next().unwrap();
        assert_eq!(command.name(), "ask");
        assert_eq!(command.signature_label(), "ask(string) -> integer");
        assert_eq!(command.documentation(), Some("Ask the current player."));
        schema
            .validate_call("ask", &[Value::String("Ready?".into())])
            .unwrap();
        schema
            .validate_reply("ask", Some(&Value::Integer(1)))
            .unwrap();
        assert!(schema.validate_call("ask", &[Value::Integer(1)]).is_err());
        assert!(
            schema
                .validate_reply("ask", Some(&Value::Boolean(true)))
                .is_err()
        );
    }
}
