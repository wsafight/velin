//! The lowering target: [`CompiledScript`] and host-command interning.
//!
//! A [`CompiledScript`] is everything an embedder needs to run a `.velin` file:
//! the bytecode [`Program`] to feed a `Machine`, the `host_id → command-name`
//! table so the embedder can dispatch [`velin_compile::Op::Host`] effects, the
//! label table for external jumps, and the `default` values to seed before
//! running.
//!
//! Host commands are interned here rather than baked in as keywords. A
//! `perform foo(...)` statement simply interns `"foo"` to a small integer.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;
use velin_check::{Environment, Type, TypeCheckSite, check_program_types, definite_assignment};
use velin_compile::{Pc, Program};
use velin_syntax::{Diagnostic, Value};

/// A parsed, lowered, runnable script.
#[derive(Debug, Clone)]
pub struct CompiledScript {
    /// The bytecode program; feed directly to `velin_vm::Machine::new`.
    pub program: Arc<Program>,
    /// `host_id → command name`; index `i` is the name of host command `i`.
    pub hosts: Vec<String>,
    /// `label name → program-counter target`, for host-driven external jumps.
    pub labels: BTreeMap<String, Pc>,
    /// `default` variables and their compile-time-evaluated values, to seed
    /// before running (`Machine::set_variable`).
    pub defaults: BTreeMap<String, Value>,
    /// Source expressions retained at their program counters for CFG-aware
    /// type propagation and precise diagnostics.
    pub(crate) type_sites: Vec<TypeCheckSite>,
}

impl CompiledScript {
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
        let mut diagnostics = Vec::new();

        let preset: BTreeSet<String> = self.defaults.keys().cloned().collect();
        for finding in definite_assignment(&self.program, &preset) {
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

        let env: Environment = self
            .defaults
            .iter()
            .map(|(name, value)| (name.clone(), Type::from(value)))
            .collect();
        diagnostics.extend(check_program_types(
            &self.program,
            &env,
            &self.type_sites,
            file,
        ));

        diagnostics
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
