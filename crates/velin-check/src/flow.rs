//! Control-flow-aware type propagation for surface-language expressions.

use crate::cfg::ControlFlow;
use crate::types::infer_with;
use crate::{Environment, Type, TypeError};
use std::collections::{BTreeMap, VecDeque};
use velin_compile::{Op, Program};
use velin_syntax::{Diagnostic, Expr};

/// How an expression participates in program type checking.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TypeCheckKind {
    /// A value read for a host argument or another non-assigning operation.
    Expression,
    /// A boolean control-flow guard.
    Condition,
    /// The right-hand side of the `Set` instruction at the same program counter.
    Assignment,
}

/// A retained source expression associated with a control-flow instruction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypeCheckSite {
    pub pc: usize,
    pub expression: Expr,
    pub kind: TypeCheckKind,
}

/// Static type contract for one opaque host command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostSignature {
    arguments: Vec<Type>,
    variadic: Option<Type>,
    returns: Option<Type>,
}

impl HostSignature {
    /// Creates a command with an exact argument list.
    #[must_use]
    pub fn exact(arguments: impl Into<Vec<Type>>, returns: Option<Type>) -> Self {
        Self {
            arguments: arguments.into(),
            variadic: None,
            returns,
        }
    }

    /// Creates a command with fixed leading arguments followed by zero or more
    /// arguments of `variadic` type.
    #[must_use]
    pub fn variadic(
        arguments: impl Into<Vec<Type>>,
        variadic: Type,
        returns: Option<Type>,
    ) -> Self {
        Self {
            arguments: arguments.into(),
            variadic: Some(variadic),
            returns,
        }
    }

    #[must_use]
    pub fn accepts(&self, count: usize) -> bool {
        count == self.arguments.len() || (self.variadic.is_some() && count >= self.arguments.len())
    }

    #[must_use]
    pub fn argument(&self, index: usize) -> Option<Type> {
        self.arguments.get(index).copied().or(self.variadic)
    }

    #[must_use]
    pub const fn returns(&self) -> Option<Type> {
        self.returns
    }

    #[must_use]
    pub fn minimum_arguments(&self) -> usize {
        self.arguments.len()
    }

    #[must_use]
    pub fn is_variadic(&self) -> bool {
        self.variadic.is_some()
    }
}

/// Host signatures keyed by the program-local `host_id`.
pub type HostSignatures = BTreeMap<u32, HostSignature>;

/// Propagates variable types through the program CFG and checks reachable sites.
///
/// Concrete types survive a merge only when all incoming paths agree. A host
/// binding or conflicting assignments produce `Unknown`, preserving the
/// checker's no-guessing rule.
#[must_use]
pub fn check_program_types(
    program: &Program,
    initial: &Environment,
    sites: &[TypeCheckSite],
    file: &str,
) -> Vec<Diagnostic> {
    check_program_types_with_hosts(program, initial, sites, &HostSignatures::new(), file)
}

/// Propagates types like [`check_program_types`], using host return contracts
/// for bound effects and validating host argument types at each call site.
#[must_use]
pub fn check_program_types_with_hosts(
    program: &Program,
    initial: &Environment,
    sites: &[TypeCheckSite],
    hosts: &HostSignatures,
    file: &str,
) -> Vec<Diagnostic> {
    if program.ops.is_empty() {
        return Vec::new();
    }

    let sites_by_pc = index_sites(program, sites);
    let entry = entry_state(program, initial);

    let flow = ControlFlow::new(&program.ops);
    let mut incoming = vec![None; flow.blocks.len()];
    incoming[0] = Some(entry);
    let mut pending = VecDeque::from([0usize]);
    let mut queued = vec![false; flow.blocks.len()];
    queued[0] = true;
    while let Some(block_id) = pending.pop_front() {
        queued[block_id] = false;
        let Some(mut outgoing) = incoming[block_id].clone() else {
            continue;
        };
        let block = flow.blocks[block_id];
        for (pc, pc_sites) in sites_by_pc
            .iter()
            .enumerate()
            .take(block.end)
            .skip(block.start)
        {
            transfer(&program.ops[pc], pc_sites, program, hosts, &mut outgoing);
        }
        for successor in &flow.successors[block_id] {
            if merge_into(&mut incoming[*successor], &outgoing) && !queued[*successor] {
                queued[*successor] = true;
                pending.push_back(*successor);
            }
        }
    }

    let mut diagnostics = Vec::new();
    for (block_id, block) in flow.blocks.iter().enumerate() {
        if !flow.reachable[block_id] {
            continue;
        }
        let Some(mut state) = incoming[block_id].clone() else {
            continue;
        };
        for (pc, pc_sites) in sites_by_pc
            .iter()
            .enumerate()
            .take(block.end)
            .skip(block.start)
        {
            let mut host_argument = 0;
            for site in pc_sites {
                let (inferred, errors) = check_site(site, program, &state);
                diagnostics.extend(errors.into_iter().map(|error| {
                    let (line, column) = error.span.map_or((0, 1), |span| (span.line, span.column));
                    Diagnostic::new(file, line, column, error.message)
                }));
                if site.kind == TypeCheckKind::Expression
                    && let Op::Host { host_id, .. } = &program.ops[pc]
                {
                    if let Some(expected) = hosts
                        .get(host_id)
                        .and_then(|signature| signature.argument(host_argument))
                        && !inferred.could_be(expected)
                    {
                        let span = site.expression.span();
                        diagnostics.push(Diagnostic::new(
                            file,
                            span.map_or(0, |span| span.line),
                            span.map_or(1, |span| span.column),
                            format!(
                                "host argument {} expects {}, found {}",
                                host_argument + 1,
                                expected.name(),
                                inferred.name()
                            ),
                        ));
                    }
                    host_argument += 1;
                }
            }
            transfer(&program.ops[pc], pc_sites, program, hosts, &mut state);
        }
    }
    diagnostics
}

fn index_sites<'a>(program: &Program, sites: &'a [TypeCheckSite]) -> Vec<Vec<&'a TypeCheckSite>> {
    let mut sites_by_pc = vec![Vec::new(); program.ops.len()];
    for site in sites {
        if let Some(target) = sites_by_pc.get_mut(site.pc) {
            target.push(site);
        }
    }
    sites_by_pc
}

fn entry_state(program: &Program, initial: &Environment) -> Vec<Type> {
    let mut entry = vec![Type::Unknown; program.slots.len()];
    for (name, kind) in initial {
        if let Some(slot) = program.slots.get(name) {
            entry[slot as usize] = *kind;
        }
    }
    entry
}

fn transfer(
    op: &Op,
    sites: &[&TypeCheckSite],
    program: &Program,
    hosts: &HostSignatures,
    state: &mut [Type],
) {
    match op {
        Op::Set { slot, .. } => {
            let Some(expression) = sites
                .iter()
                .find(|site| site.kind == TypeCheckKind::Assignment)
                .map(|site| &site.expression)
            else {
                return;
            };
            let inferred = infer_in_state(expression, program, state, &mut Vec::new());
            if let Some(target) = state.get_mut(*slot as usize) {
                *target = inferred;
            }
        }
        Op::Host {
            host_id,
            bind: Some(slot),
            ..
        } => {
            if let Some(target) = state.get_mut(*slot as usize) {
                *target = hosts
                    .get(host_id)
                    .and_then(HostSignature::returns)
                    .unwrap_or(Type::Unknown);
            }
        }
        Op::Jump(_) | Op::JumpIfFalse { .. } | Op::Host { bind: None, .. } | Op::Halt => {}
    }
}

fn check_site(site: &TypeCheckSite, program: &Program, state: &[Type]) -> (Type, Vec<TypeError>) {
    let mut errors = Vec::new();
    let inferred = infer_in_state(&site.expression, program, state, &mut errors);
    if site.kind == TypeCheckKind::Condition && !matches!(inferred, Type::Boolean | Type::Unknown) {
        errors.push(TypeError {
            message: format!("condition expects boolean, found {}", inferred.name()),
            span: site.expression.span().cloned(),
        });
    }
    (inferred, errors)
}

fn infer_in_state(
    expression: &Expr,
    program: &Program,
    state: &[Type],
    errors: &mut Vec<TypeError>,
) -> Type {
    infer_with(
        expression,
        &|name| {
            program
                .slots
                .get(name)
                .and_then(|slot| state.get(slot as usize))
                .copied()
                .unwrap_or(Type::Unknown)
        },
        errors,
    )
}

fn merge_into(target: &mut Option<Vec<Type>>, incoming: &[Type]) -> bool {
    let Some(current) = target else {
        *target = Some(incoming.to_vec());
        return true;
    };
    let mut changed = false;
    for (current, incoming) in current.iter_mut().zip(incoming) {
        let merged = if *current == *incoming {
            *current
        } else {
            Type::Unknown
        };
        if merged != *current {
            *current = merged;
            changed = true;
        }
    }
    changed
}
