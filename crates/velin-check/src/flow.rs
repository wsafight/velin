//! Control-flow-aware type propagation for surface-language expressions.

use crate::{Environment, Type, check_condition, check_expression, infer};
use std::collections::{BTreeMap, HashMap, VecDeque};
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

    let mut assignments: Vec<Option<&Expr>> = vec![None; program.ops.len()];
    for site in sites {
        if site.kind == TypeCheckKind::Assignment
            && let Some(target) = assignments.get_mut(site.pc)
        {
            *target = Some(&site.expression);
        }
    }

    let mut entry = vec![Type::Unknown; program.slots.len()];
    for (name, kind) in initial {
        if let Some(slot) = program.slots.get(name) {
            entry[slot as usize] = *kind;
        }
    }

    let mut incoming = vec![None; program.ops.len()];
    incoming[0] = Some(entry);
    let mut pending = VecDeque::from([0usize]);
    while let Some(pc) = pending.pop_front() {
        let Some(mut outgoing) = incoming[pc].clone() else {
            continue;
        };
        match &program.ops[pc] {
            Op::Set { slot, .. } => {
                if let Some(expression) = assignments[pc] {
                    let env = environment(program, &outgoing);
                    let inferred = infer(expression, &env, &mut Vec::new());
                    if let Some(target) = outgoing.get_mut(*slot as usize) {
                        *target = inferred;
                    }
                }
            }
            Op::Host {
                host_id,
                bind: Some(slot),
                ..
            } => {
                if let Some(target) = outgoing.get_mut(*slot as usize) {
                    *target = hosts
                        .get(host_id)
                        .and_then(HostSignature::returns)
                        .unwrap_or(Type::Unknown);
                }
            }
            _ => {}
        }
        for successor in successors(pc, &program.ops[pc]) {
            let Some(target) = incoming.get_mut(successor) else {
                continue;
            };
            if merge_into(target, &outgoing) {
                pending.push_back(successor);
            }
        }
    }

    let mut diagnostics = Vec::new();
    let mut host_argument_indices = HashMap::<usize, usize>::new();
    for site in sites {
        let Some(Some(state)) = incoming.get(site.pc) else {
            continue;
        };
        let env = environment(program, state);
        diagnostics.extend(match site.kind {
            TypeCheckKind::Condition => check_condition(&site.expression, &env, file, 0, 1),
            TypeCheckKind::Expression | TypeCheckKind::Assignment => {
                check_expression(&site.expression, &env, file, 0, 1)
            }
        });
        if site.kind == TypeCheckKind::Expression
            && let Some(Op::Host { host_id, .. }) = program.ops.get(site.pc)
        {
            let index = host_argument_indices.entry(site.pc).or_default();
            if let Some(expected) = hosts
                .get(host_id)
                .and_then(|signature| signature.argument(*index))
            {
                let actual = infer(&site.expression, &env, &mut Vec::new());
                if !actual.could_be(expected) {
                    let span = site.expression.span();
                    diagnostics.push(Diagnostic::new(
                        file,
                        span.map_or(0, |span| span.line),
                        span.map_or(1, |span| span.column),
                        format!(
                            "host argument {} expects {}, found {}",
                            *index + 1,
                            expected.name(),
                            actual.name()
                        ),
                    ));
                }
            }
            *index += 1;
        }
    }
    diagnostics
}

fn environment(program: &Program, state: &[Type]) -> Environment {
    program
        .slots
        .names()
        .iter()
        .zip(state)
        .filter(|(_, kind)| **kind != Type::Unknown)
        .map(|(name, kind)| (name.clone(), *kind))
        .collect()
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

fn successors(pc: usize, op: &Op) -> Vec<usize> {
    match op {
        Op::Jump(target) => vec![*target as usize],
        Op::JumpIfFalse { target, .. } => vec![pc + 1, *target as usize],
        Op::Halt => Vec::new(),
        Op::Set { .. } | Op::Host { .. } => vec![pc + 1],
    }
}
