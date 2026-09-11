//! Control-flow-aware type propagation for surface-language expressions.

use crate::{Environment, Type, check_condition, check_expression, infer};
use std::collections::VecDeque;
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
                bind: Some(slot), ..
            } => {
                if let Some(target) = outgoing.get_mut(*slot as usize) {
                    *target = Type::Unknown;
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
