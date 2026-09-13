//! Definite-assignment analysis over a compiled [`Program`].
//!
//! A Velin program reads variables from frame slots. Reading a slot that was
//! never assigned on the current path is a runtime error ("has not been
//! assigned on this path"). This is a classic *must* dataflow analysis that
//! proves, for every read, that the slot is assigned on **all** control-flow
//! paths reaching it — turning that runtime error into a compile-time
//! diagnostic.
//!
//! The analysis is intraprocedural and conservative in the safe direction: it
//! only reports a read it can prove is always unassigned, so it never rejects a
//! program the runtime would have accepted.

use crate::cfg::{ControlFlow, is_straight_line};
use std::collections::{BTreeSet, VecDeque};
use velin_bytecode::{ExprChunkRef, ExprOp, Op, Program, UpdateOp};
use velin_syntax::{BinaryOp, Builtin, UnaryOp, Value};

mod expression;

#[cfg(test)]
use expression::{AbstractStack, AbstractValue, INLINE_ABSTRACT_VALUES};
use expression::{ExpressionWorkspace, LoadSet, reachable_loads};

/// A use of a slot the analysis proved is unassigned on some path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnassignedUse {
    pub name: String,
    pub line: usize,
    pub column: usize,
}

/// Runs definite-assignment analysis over `program`, treating `preset` slot
/// names (e.g. `default` variables the host seeds before running) as assigned
/// on entry.
///
/// Returns every read that is not definitely assigned, in program order.
#[must_use]
pub fn definite_assignment(program: &Program, preset: &BTreeSet<String>) -> Vec<UnassignedUse> {
    let slots: Vec<_> = preset
        .iter()
        .filter_map(|name| program.slots.get(name))
        .collect();
    definite_assignment_slots(program, &slots)
}

/// Runs definite-assignment analysis with entry state already resolved to
/// frame slots.
#[must_use]
pub fn definite_assignment_slots(program: &Program, preset: &[u32]) -> Vec<UnassignedUse> {
    let slot_count = program.slots.len();
    if program.ops.is_empty() {
        return Vec::new();
    }

    // `assigned_in[pc]` = slots definitely assigned when control reaches `pc`.
    // Must-analysis: initialise every non-entry block to "all slots" (top) and
    // iterate the intersection to a fixpoint. The entry starts from `preset`.
    let entry: BitSet = preset
        .iter()
        .fold(BitSet::empty(slot_count), |mut set, slot| {
            set.insert(*slot);
            set
        });

    if is_straight_line(&program.ops) {
        return find_unassigned_linear(program, entry);
    }

    let flow = ControlFlow::new(&program.ops);

    let mut assigned_out = vec![BitSet::full(slot_count); flow.blocks.len()];
    let mut pending = VecDeque::from([0]);
    let mut queued = vec![false; flow.blocks.len()];
    queued[0] = true;
    while let Some(block_id) = pending.pop_front() {
        queued[block_id] = false;
        let mut outgoing = incoming_set(block_id, &flow, &assigned_out, &entry);
        let block = flow.blocks[block_id];
        for op in &program.ops[block.start..block.end] {
            if let Some(slot) = assigned_slot(op) {
                outgoing.insert(slot);
            }
        }
        if outgoing != assigned_out[block_id] {
            assigned_out[block_id] = outgoing;
            for successor in &flow.successors[block_id] {
                if !queued[*successor] {
                    queued[*successor] = true;
                    pending.push_back(*successor);
                }
            }
        }
    }

    find_unassigned_reads(program, &flow, &assigned_out, &entry)
}

fn find_unassigned_reads(
    program: &Program,
    flow: &ControlFlow,
    assigned_out: &[BitSet],
    entry: &BitSet,
) -> Vec<UnassignedUse> {
    let mut findings = Vec::new();
    let mut load_cache: Vec<LoadSet> = std::iter::repeat_with(|| LoadSet::Uncomputed)
        .take(program.chunks.len())
        .collect();
    let mut expression_workspace = ExpressionWorkspace::default();
    for (block_id, block) in flow.blocks.iter().enumerate() {
        if !flow.reachable[block_id] {
            continue;
        }
        let mut assigned = incoming_set(block_id, flow, assigned_out, entry);
        for op in &program.ops[block.start..block.end] {
            record_reads(
                op,
                program,
                &assigned,
                &mut load_cache,
                &mut expression_workspace,
                &mut findings,
            );
            if let Some(slot) = assigned_slot(op) {
                assigned.insert(slot);
            }
        }
    }
    findings
}

fn find_unassigned_linear(program: &Program, mut assigned: BitSet) -> Vec<UnassignedUse> {
    let mut findings = Vec::new();
    let mut load_cache: Vec<LoadSet> = std::iter::repeat_with(|| LoadSet::Uncomputed)
        .take(program.chunks.len())
        .collect();
    let mut expression_workspace = ExpressionWorkspace::default();
    for op in &program.ops {
        record_reads(
            op,
            program,
            &assigned,
            &mut load_cache,
            &mut expression_workspace,
            &mut findings,
        );
        if let Some(slot) = assigned_slot(op) {
            assigned.insert(slot);
        }
    }
    findings
}

fn record_reads(
    op: &Op,
    program: &Program,
    assigned: &BitSet,
    load_cache: &mut [LoadSet],
    expression_workspace: &mut ExpressionWorkspace,
    findings: &mut Vec<UnassignedUse>,
) {
    match op {
        Op::Set { value, .. } => {
            record_unassigned(
                *value,
                program,
                assigned,
                load_cache,
                expression_workspace,
                findings,
            );
        }
        Op::CopySlot {
            source,
            line,
            column,
            ..
        } => {
            if !assigned.contains(*source) {
                findings.push(UnassignedUse {
                    name: program.slots.name(*source).unwrap_or("?").to_owned(),
                    line: *line as usize,
                    column: *column as usize,
                });
            }
        }
        Op::Update {
            slot,
            operation,
            line,
            column,
        } => {
            if !assigned.contains(*slot) {
                findings.push(UnassignedUse {
                    name: program.slots.name(*slot).unwrap_or("?").to_owned(),
                    line: *line as usize,
                    column: *column as usize,
                });
            }
            for chunk in update_chunks(*operation).into_iter().flatten() {
                record_unassigned(
                    chunk,
                    program,
                    assigned,
                    load_cache,
                    expression_workspace,
                    findings,
                );
            }
        }
        Op::JumpIfFalse { condition, .. } | Op::JumpIfIntegerCompare { condition, .. } => {
            record_unassigned(
                *condition,
                program,
                assigned,
                load_cache,
                expression_workspace,
                findings,
            );
        }
        Op::Host(host) => {
            for chunk in &host.args {
                record_unassigned(
                    *chunk,
                    program,
                    assigned,
                    load_cache,
                    expression_workspace,
                    findings,
                );
            }
        }
        Op::SetConst { .. } | Op::Jump(_) | Op::Halt => {}
    }
}

/// The set of slots definitely assigned on entry to `pc`: the intersection of
/// its predecessors' out-sets, or the program `entry` set for the root.
fn incoming_set(
    block_id: usize,
    flow: &ControlFlow,
    assigned_out: &[BitSet],
    entry: &BitSet,
) -> BitSet {
    let mut incoming = (block_id == 0).then(|| entry.clone());
    for predecessor in flow.predecessors[block_id]
        .iter()
        .copied()
        .filter(|predecessor| flow.reachable[*predecessor])
    {
        if let Some(current) = &mut incoming {
            current.intersect_assign(&assigned_out[predecessor]);
        } else {
            incoming = Some(assigned_out[predecessor].clone());
        }
    }
    incoming.unwrap_or_else(|| entry.clone())
}

fn assigned_slot(op: &Op) -> Option<u32> {
    match op {
        Op::Set { slot, .. }
        | Op::SetConst { slot, .. }
        | Op::CopySlot { slot, .. }
        | Op::Update { slot, .. } => Some(*slot),
        Op::Host(host) => host.bind,
        _ => None,
    }
}

fn update_chunks(operation: UpdateOp) -> [Option<u32>; 2] {
    match operation {
        UpdateOp::Add { rhs } => [Some(rhs), None],
        UpdateOp::AddInteger { .. } => [None, None],
        UpdateOp::Push { value } => [Some(value), None],
        UpdateOp::Put { key, value } => [Some(key), Some(value)],
        UpdateOp::Remove { key } => [Some(key), None],
    }
}

fn record_unassigned(
    chunk_id: u32,
    program: &Program,
    assigned: &BitSet,
    load_cache: &mut [LoadSet],
    expression_workspace: &mut ExpressionWorkspace,
    findings: &mut Vec<UnassignedUse>,
) {
    let Some(chunk) = program.chunk(chunk_id) else {
        return;
    };
    let Some(cache) = load_cache.get_mut(chunk_id as usize) else {
        return;
    };
    if matches!(cache, LoadSet::Uncomputed) {
        *cache = reachable_loads(chunk, expression_workspace);
    }
    for (slot, column) in cache.as_slice() {
        if !assigned.contains(*slot) {
            findings.push(UnassignedUse {
                name: program.slots.name(*slot).unwrap_or("?").to_owned(),
                line: chunk.line as usize,
                column: *column,
            });
        }
    }
}

/// A tiny fixed-width bit set over slot indices. Kept local to avoid a
/// dependency; slot counts are small (bounded by the data budget).
#[derive(Debug, Clone, PartialEq, Eq)]
struct BitSet {
    bits: Vec<u64>,
    len: usize,
}

impl BitSet {
    fn empty(len: usize) -> Self {
        Self {
            bits: vec![0; len.div_ceil(64)],
            len,
        }
    }

    fn full(len: usize) -> Self {
        let mut set = Self {
            bits: vec![u64::MAX; len.div_ceil(64)],
            len,
        };
        // Clear padding bits beyond `len` so equality is well-defined.
        for slot in len..set.bits.len() * 64 {
            set.remove(u32::try_from(slot).expect("slot fits in u32"));
        }
        set
    }

    fn insert(&mut self, slot: u32) {
        let slot = slot as usize;
        if slot < self.len {
            self.bits[slot / 64] |= 1 << (slot % 64);
        }
    }

    fn remove(&mut self, slot: u32) {
        let slot = slot as usize;
        if slot / 64 < self.bits.len() {
            self.bits[slot / 64] &= !(1 << (slot % 64));
        }
    }

    fn contains(&self, slot: u32) -> bool {
        let slot = slot as usize;
        slot < self.len && (self.bits[slot / 64] >> (slot % 64)) & 1 == 1
    }

    fn intersect_assign(&mut self, other: &Self) {
        for (current, incoming) in self.bits.iter_mut().zip(&other.bits) {
            *current &= incoming;
        }
    }
}

#[cfg(test)]
#[path = "definite_tests.rs"]
mod tests;
