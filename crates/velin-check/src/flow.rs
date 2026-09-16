//! Control-flow-aware type propagation for surface-language expressions.

use crate::cfg::{ControlFlow, is_straight_line};
use crate::types::infer_with;
use crate::{Environment, Type, TypeError};
use std::collections::{BTreeMap, VecDeque};
use velin_bytecode::{Op, Program};
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

    /// Returns the fixed leading argument types in declaration order.
    #[must_use]
    pub fn arguments(&self) -> &[Type] {
        &self.arguments
    }

    /// Returns the repeated trailing argument type, when this is variadic.
    #[must_use]
    pub const fn variadic_type(&self) -> Option<Type> {
        self.variadic
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
    check_program_types_from_entry(program, entry_state(program, initial), sites, hosts, file)
}

/// Propagates types from a dense slot-indexed entry state.
#[must_use]
pub fn check_program_types_with_slot_types(
    program: &Program,
    initial: &[Type],
    sites: &[TypeCheckSite],
    file: &str,
) -> Vec<Diagnostic> {
    check_program_types_with_hosts_and_slot_types(
        program,
        initial,
        sites,
        &HostSignatures::new(),
        file,
    )
}

/// Propagates dense slot-indexed entry types and applies host contracts.
#[must_use]
pub fn check_program_types_with_hosts_and_slot_types(
    program: &Program,
    initial: &[Type],
    sites: &[TypeCheckSite],
    hosts: &HostSignatures,
    file: &str,
) -> Vec<Diagnostic> {
    let mut entry = vec![Type::Unknown; program.slots.len()];
    let copied = entry.len().min(initial.len());
    entry[..copied].copy_from_slice(&initial[..copied]);
    check_program_types_from_entry(program, entry, sites, hosts, file)
}

fn check_program_types_from_entry(
    program: &Program,
    entry: Vec<Type>,
    sites: &[TypeCheckSite],
    hosts: &HostSignatures,
    file: &str,
) -> Vec<Diagnostic> {
    if program.ops.is_empty() {
        return Vec::new();
    }

    let straight_line = is_straight_line(&program.ops);
    if straight_line && sites_are_ordered_and_valid(program.ops.len(), sites) {
        return check_ordered_linear(program, entry, sites, hosts, file);
    }

    let sites_by_pc = SiteIndex::new(program.ops.len(), sites);
    if straight_line {
        let mut state = entry;
        let mut diagnostics = Vec::new();
        check_range(
            program,
            0,
            program.ops.len(),
            &sites_by_pc,
            hosts,
            file,
            &mut state,
            &mut diagnostics,
        );
        return diagnostics;
    }

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
        for pc in block.start..block.end {
            let pc_sites = sites_by_pc.at(pc);
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
        check_range(
            program,
            block.start,
            block.end,
            &sites_by_pc,
            hosts,
            file,
            &mut state,
            &mut diagnostics,
        );
    }
    diagnostics
}

fn sites_are_ordered_and_valid(op_count: usize, sites: &[TypeCheckSite]) -> bool {
    sites.iter().all(|site| site.pc < op_count)
        && sites.windows(2).all(|pair| pair[0].pc <= pair[1].pc)
}

fn check_ordered_linear(
    program: &Program,
    mut state: Vec<Type>,
    sites: &[TypeCheckSite],
    hosts: &HostSignatures,
    file: &str,
) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    let mut site_at = 0;
    for pc in 0..program.ops.len() {
        let start = site_at;
        while sites.get(site_at).is_some_and(|site| site.pc == pc) {
            site_at += 1;
        }
        check_sites(
            program,
            pc,
            sites[start..site_at].iter(),
            hosts,
            file,
            &mut state,
            &mut diagnostics,
        );
    }
    diagnostics
}

#[allow(clippy::too_many_arguments)]
fn check_range(
    program: &Program,
    start: usize,
    end: usize,
    sites_by_pc: &SiteIndex<'_>,
    hosts: &HostSignatures,
    file: &str,
    state: &mut [Type],
    diagnostics: &mut Vec<Diagnostic>,
) {
    for pc in start..end {
        check_sites(
            program,
            pc,
            sites_by_pc.at(pc).iter().copied(),
            hosts,
            file,
            state,
            diagnostics,
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn check_sites<'a>(
    program: &Program,
    pc: usize,
    sites: impl Iterator<Item = &'a TypeCheckSite>,
    hosts: &HostSignatures,
    file: &str,
    state: &mut [Type],
    diagnostics: &mut Vec<Diagnostic>,
) {
    let mut host_argument = 0;
    let mut assignment = None;
    for site in sites {
        let (inferred, errors) = check_site(site, program, state);
        if site.kind == TypeCheckKind::Assignment && assignment.is_none() {
            assignment = Some(inferred);
        }
        diagnostics.extend(errors.into_iter().map(|error| {
            let (line, column) = error.span.map_or((0, 1), |span| (span.line, span.column));
            Diagnostic::new(file, line, column, error.message)
        }));
        if site.kind == TypeCheckKind::Expression
            && let Op::Host(host) = &program.ops[pc]
        {
            if let Some(expected) = hosts
                .get(&host.host_id)
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
    apply_transfer(&program.ops[pc], assignment, hosts, state);
}

struct SiteIndex<'a> {
    sites: Vec<&'a TypeCheckSite>,
    offsets: Vec<usize>,
}

impl<'a> SiteIndex<'a> {
    fn new(op_count: usize, sites: &'a [TypeCheckSite]) -> Self {
        let mut indexed: Vec<_> = sites.iter().filter(|site| site.pc < op_count).collect();
        if indexed.windows(2).any(|pair| pair[0].pc > pair[1].pc) {
            indexed.sort_by_key(|site| site.pc);
        }
        let mut offsets = vec![0; op_count + 1];
        for site in &indexed {
            offsets[site.pc + 1] += 1;
        }
        for pc in 1..offsets.len() {
            offsets[pc] += offsets[pc - 1];
        }
        Self {
            sites: indexed,
            offsets,
        }
    }

    fn at(&self, pc: usize) -> &[&'a TypeCheckSite] {
        &self.sites[self.offsets[pc]..self.offsets[pc + 1]]
    }
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
    let assignment = match op {
        Op::Set { .. } | Op::SetConst { .. } | Op::CopySlot { .. } | Op::Update { .. } => sites
            .iter()
            .find(|site| site.kind == TypeCheckKind::Assignment)
            .map(|site| &site.expression)
            .map(|expression| infer_in_state(expression, program, state, &mut Vec::new())),
        _ => None,
    };
    apply_transfer(op, assignment, hosts, state);
}

fn apply_transfer(op: &Op, assignment: Option<Type>, hosts: &HostSignatures, state: &mut [Type]) {
    match op {
        Op::Set { slot, .. }
        | Op::SetConst { slot, .. }
        | Op::CopySlot { slot, .. }
        | Op::Update { slot, .. } => {
            if let (Some(target), Some(inferred)) = (state.get_mut(*slot as usize), assignment) {
                *target = inferred;
            }
        }
        Op::Host(host) if host.bind.is_some() => {
            let slot = host.bind.expect("guard checked host binding");
            if let Some(target) = state.get_mut(slot as usize) {
                *target = hosts
                    .get(&host.host_id)
                    .and_then(HostSignature::returns)
                    .unwrap_or(Type::Unknown);
            }
        }
        Op::Jump(_)
        | Op::JumpIfFalse { .. }
        | Op::JumpIfIntegerCompare { .. }
        | Op::Host(_)
        | Op::Halt => {}
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
