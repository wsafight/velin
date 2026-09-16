//! The lowering target: [`CompiledScript`] and host-command interning.
//!
//! A [`CompiledScript`] is everything an embedder needs to run a `.velin` file:
//! the bytecode [`Program`] to feed a `Machine`, the `host_id → command-name`
//! table so the embedder can dispatch [`velin_bytecode::Op::Host`] effects, the
//! label table for external jumps, and the `default` values to seed before
//! running.
//!
//! Host commands are interned here rather than baked in as keywords. A
//! `perform foo(...)` statement simply interns `"foo"` to a small integer.

use crate::HostSchema;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;
use velin_bytecode::{InitialFrame, Pc, Program, ValidatedProgram};
use velin_check::{
    Environment, HostSignatures, Type, TypeCheckSite, check_program_types,
    check_program_types_with_hosts, check_program_types_with_hosts_and_slot_types,
    check_program_types_with_slot_types, definite_assignment, definite_assignment_slots,
};
use velin_syntax::{Diagnostic, Value};

/// A parsed, lowered, runnable script.
#[derive(Debug, Clone)]
pub struct CompiledScript {
    /// The bytecode program; feed directly to `velin_vm::Machine::new`.
    pub program: Arc<Program>,
    pub(crate) validated_program: ValidatedProgram,
    /// `host_id → command name`; index `i` is the name of host command `i`.
    pub hosts: Vec<String>,
    /// `label name → program-counter target`, for host-driven external jumps.
    pub labels: BTreeMap<String, Pc>,
    /// `default` variables and their compile-time-evaluated values, to seed
    /// before running (`Machine::set_variable`).
    pub defaults: BTreeMap<String, Value>,
    /// Dense initialization state prepared from the original defaults.
    pub(crate) initial_frame: InitialFrame,
    pub(crate) initial_types: Box<[Type]>,
    /// Source expressions retained at their program counters for CFG-aware
    /// type propagation and precise diagnostics.
    pub(crate) type_sites: Vec<TypeCheckSite>,
    pub(crate) host_sites: Vec<HostCheckSite>,
}

#[derive(Debug, Clone)]
pub(crate) struct HostCheckSite {
    pub host_id: u32,
    pub arguments: usize,
    pub bind: bool,
    pub line: usize,
}

impl CompiledScript {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn from_artifact(
        program: Arc<Program>,
        validated_program: ValidatedProgram,
        hosts: Vec<String>,
        labels: BTreeMap<String, Pc>,
        defaults: BTreeMap<String, Value>,
        initial_frame: InitialFrame,
        initial_types: Box<[Type]>,
        type_sites: Vec<TypeCheckSite>,
        host_sites: Vec<HostCheckSite>,
    ) -> Self {
        Self {
            program,
            validated_program,
            hosts,
            labels,
            defaults,
            initial_frame,
            initial_types,
            type_sites,
            host_sites,
        }
    }

    /// Returns the validation proof shared by fast machine constructors.
    #[must_use]
    pub const fn validated_program(&self) -> &ValidatedProgram {
        &self.validated_program
    }

    /// Returns the prepared initial frame while the public defaults still
    /// match the values from compilation.
    #[must_use]
    pub fn initial_frame(&self) -> Option<&InitialFrame> {
        self.initial_frame
            .matches_named_values(
                &self.validated_program.program().slots,
                self.defaults
                    .iter()
                    .map(|(name, value)| (name.as_str(), value)),
            )
            .then_some(&self.initial_frame)
    }

    /// Returns the command name a `host_id` was interned from.
    #[must_use]
    pub fn host_name(&self, host_id: u32) -> Option<&str> {
        self.hosts.get(host_id as usize).map(String::as_str)
    }

    /// Runs Velin's conservative static checks: definite assignment over the
    /// program (with `default`s treated as assigned on entry) and expression
    /// type inference propagated through the same control-flow graph. Concrete
    /// types survive a merge only when every incoming path agrees. Diagnostics
    /// are attributed to `file`.
    #[must_use]
    pub fn check(&self, file: &str) -> Vec<Diagnostic> {
        self.check_inner(file, None, None)
    }

    /// Runs static checks with host-owned command contracts.
    #[must_use]
    pub fn check_with_host_schema(&self, file: &str, schema: &HostSchema) -> Vec<Diagnostic> {
        self.check_inner(file, Some(schema), None)
    }

    /// Runs static checks with values supplied by an embedding session treated
    /// as assigned entry state. Names absent from this program are ignored.
    /// This is used by incremental frontends such as a REPL without changing
    /// the program's immutable defaults.
    #[must_use]
    pub fn check_with_values(
        &self,
        file: &str,
        values: &BTreeMap<String, Value>,
    ) -> Vec<Diagnostic> {
        self.check_inner(file, None, Some(values))
    }

    /// Runs static checks with externally supplied variable types treated as
    /// assigned entry state. The compiled defaults remain available for names
    /// that are not present in `bindings`; binding types override a default's
    /// type for the duration of this check.
    #[must_use]
    pub fn check_with_bindings(
        &self,
        file: &str,
        bindings: &BTreeMap<String, Type>,
    ) -> Vec<Diagnostic> {
        self.check_with_bindings_and_schema(file, bindings, None)
    }

    /// Runs static checks with externally supplied variable types and host
    /// command contracts. This is the combined entry point used by embedders
    /// whose inputs and host protocol are both known before invocation.
    #[must_use]
    pub fn check_with_bindings_and_host_schema(
        &self,
        file: &str,
        bindings: &BTreeMap<String, Type>,
        schema: &HostSchema,
    ) -> Vec<Diagnostic> {
        self.check_with_bindings_and_schema(file, bindings, Some(schema))
    }

    fn check_with_bindings_and_schema(
        &self,
        file: &str,
        bindings: &BTreeMap<String, Type>,
        schema: Option<&HostSchema>,
    ) -> Vec<Diagnostic> {
        let mut assigned = Vec::new();
        let mut types = vec![Type::Unknown; self.program.slots.len()];

        // Defaults are part of the normal entry frame. If the public defaults
        // were edited after compilation, fall back to their current values so
        // diagnostics still describe the script that will be instantiated.
        if let Some(frame) = self
            .validated_program
            .refers_to(&self.program)
            .then(|| self.initial_frame())
            .flatten()
        {
            assigned.extend_from_slice(frame.assigned_slots());
            let copied = types.len().min(self.initial_types.len());
            types[..copied].copy_from_slice(&self.initial_types[..copied]);
        } else {
            for (name, value) in &self.defaults {
                if let Some(slot) = self.program.slots.get(name) {
                    assigned.push(slot);
                    types[slot as usize] = Type::from(value);
                }
            }
        }

        for (name, ty) in bindings {
            if let Some(slot) = self.program.slots.get(name) {
                assigned.push(slot);
                types[slot as usize] = *ty;
            }
        }

        self.check_inner_context(file, schema, Some(&assigned), Some(&types))
    }

    #[allow(clippy::too_many_lines)]
    fn check_inner(
        &self,
        file: &str,
        schema: Option<&HostSchema>,
        bindings: Option<&BTreeMap<String, Value>>,
    ) -> Vec<Diagnostic> {
        let prepared = if bindings.is_none() {
            self.validated_program
                .refers_to(&self.program)
                .then(|| self.initial_frame())
                .flatten()
        } else {
            None
        };
        let mut context_assigned = None;
        let mut context_types = None;
        if let Some(values) = bindings {
            let mut assigned = Vec::new();
            let mut types = vec![Type::Unknown; self.program.slots.len()];
            for (name, value) in values {
                if let Some(slot) = self.program.slots.get(name) {
                    assigned.push(slot);
                    types[slot as usize] = Type::from(value);
                }
            }
            context_assigned = Some(assigned);
            context_types = Some(types);
        } else if let Some(frame) = prepared {
            context_assigned = Some(frame.assigned_slots().to_vec());
            context_types = Some(self.initial_types.to_vec());
        }
        self.check_inner_context(
            file,
            schema,
            context_assigned.as_deref(),
            context_types.as_deref(),
        )
    }

    #[allow(clippy::too_many_lines)]
    fn check_inner_context(
        &self,
        file: &str,
        schema: Option<&HostSchema>,
        context_assigned: Option<&[u32]>,
        context_types: Option<&[Type]>,
    ) -> Vec<Diagnostic> {
        let mut diagnostics = Vec::new();
        let unassigned = if let Some(assigned) = context_assigned {
            definite_assignment_slots(&self.program, assigned)
        } else {
            let preset: BTreeSet<String> = self.defaults.keys().cloned().collect();
            definite_assignment(&self.program, &preset)
        };
        for finding in unassigned {
            diagnostics.push(Diagnostic::new(
                file,
                finding.line,
                finding.column,
                format!(
                    "variable `{}` may be read before it is assigned on some path",
                    finding.name
                ),
            ));
        }

        if let Some(schema) = schema {
            let mut signatures = HostSignatures::new();
            for (host_id, name) in self.hosts.iter().enumerate() {
                if let (Ok(host_id), Some(signature)) = (u32::try_from(host_id), schema.get(name)) {
                    signatures.insert(host_id, signature.clone());
                }
            }
            diagnostics.extend(if let Some(types) = context_types {
                check_program_types_with_hosts_and_slot_types(
                    &self.program,
                    types,
                    &self.type_sites,
                    &signatures,
                    file,
                )
            } else {
                let env = self.default_environment();
                check_program_types_with_hosts(
                    &self.program,
                    &env,
                    &self.type_sites,
                    &signatures,
                    file,
                )
            });
            for site in &self.host_sites {
                let name = self.host_name(site.host_id).unwrap_or("<unknown>");
                match schema.get(name) {
                    None if !schema.allows_unknown() => diagnostics.push(Diagnostic::new(
                        file,
                        site.line,
                        1,
                        format!("host command `{name}` is not declared"),
                    )),
                    Some(signature) if !signature.accepts(site.arguments) => {
                        let expected = if signature.is_variadic() {
                            format!("at least {}", signature.minimum_arguments())
                        } else {
                            signature.minimum_arguments().to_string()
                        };
                        diagnostics.push(Diagnostic::new(
                            file,
                            site.line,
                            1,
                            format!(
                                "host command `{name}` expects {expected} argument(s), found {}",
                                site.arguments
                            ),
                        ));
                    }
                    Some(signature) if site.bind && signature.returns().is_none() => {
                        diagnostics.push(Diagnostic::new(
                            file,
                            site.line,
                            1,
                            format!("host command `{name}` does not return a value"),
                        ));
                    }
                    _ => {}
                }
            }
        } else {
            diagnostics.extend(if let Some(types) = context_types {
                check_program_types_with_slot_types(&self.program, types, &self.type_sites, file)
            } else {
                let env = self.default_environment();
                check_program_types(&self.program, &env, &self.type_sites, file)
            });
        }

        diagnostics
    }

    fn default_environment(&self) -> Environment {
        self.defaults
            .iter()
            .map(|(name, value)| (name.clone(), Type::from(value)))
            .collect()
    }
}

/// Compile-time interning of host-command names to dense ids.
#[derive(Debug, Default)]
pub(crate) struct HostTable {
    names: Vec<String>,
    index: HashMap<String, u32>,
}

impl HostTable {
    /// Returns the id for `name`, allocating one on first use.
    ///
    /// # Panics
    /// Panics only if more than `u32::MAX` distinct host commands are interned,
    /// which no realistic script reaches.
    pub fn intern(&mut self, name: &str) -> u32 {
        if let Some(id) = self.index.get(name) {
            return *id;
        }
        let id = u32::try_from(self.names.len()).expect("host id fits in u32");
        self.names.push(name.to_owned());
        self.index.insert(name.to_owned(), id);
        id
    }

    /// Consumes the table, returning `host_id → name` in id order.
    pub fn into_names(self) -> Vec<String> {
        self.names
    }
}

#[cfg(test)]
#[path = "host_coverage_tests.rs"]
mod coverage_tests;
