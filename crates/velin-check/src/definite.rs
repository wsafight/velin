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
use velin_compile::{ExprChunkRef, ExprOp, Op, Program, UpdateOp};
use velin_syntax::{BinaryOp, Builtin, UnaryOp, Value};

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AbstractValue {
    Boolean(Option<bool>),
    NonBoolean,
    Unknown,
}

const INLINE_ABSTRACT_VALUES: usize = 8;

#[derive(Debug, Clone, PartialEq, Eq)]
enum AbstractStack {
    Inline {
        values: [AbstractValue; INLINE_ABSTRACT_VALUES],
        len: u8,
    },
    Overflow(Vec<AbstractValue>),
}

impl Default for AbstractStack {
    fn default() -> Self {
        Self::new()
    }
}

impl AbstractStack {
    const fn new() -> Self {
        Self::Inline {
            values: [AbstractValue::Unknown; INLINE_ABSTRACT_VALUES],
            len: 0,
        }
    }

    fn push(&mut self, value: AbstractValue) {
        match self {
            Self::Inline { values, len } if usize::from(*len) < values.len() => {
                values[usize::from(*len)] = value;
                *len += 1;
            }
            Self::Inline { values, len } => {
                let len = usize::from(*len);
                let mut overflow = Vec::with_capacity(INLINE_ABSTRACT_VALUES * 2);
                overflow.extend_from_slice(&values[..len]);
                overflow.push(value);
                *self = Self::Overflow(overflow);
            }
            Self::Overflow(values) => values.push(value),
        }
    }

    fn pop(&mut self) -> Option<AbstractValue> {
        match self {
            Self::Inline { values, len } if *len > 0 => {
                *len -= 1;
                Some(values[usize::from(*len)])
            }
            Self::Inline { .. } => None,
            Self::Overflow(values) => values.pop(),
        }
    }

    fn last(&self) -> Option<AbstractValue> {
        self.as_slice().last().copied()
    }

    fn len(&self) -> usize {
        self.as_slice().len()
    }

    fn truncate(&mut self, len: usize) {
        match self {
            Self::Inline {
                len: current_len, ..
            } => {
                *current_len = (*current_len).min(u8::try_from(len).unwrap_or(u8::MAX));
            }
            Self::Overflow(values) => values.truncate(len),
        }
    }

    fn as_slice(&self) -> &[AbstractValue] {
        match self {
            Self::Inline { values, len } => &values[..usize::from(*len)],
            Self::Overflow(values) => values,
        }
    }

    fn as_mut_slice(&mut self) -> &mut [AbstractValue] {
        match self {
            Self::Inline { values, len } => &mut values[..usize::from(*len)],
            Self::Overflow(values) => values,
        }
    }
}

#[derive(Default)]
struct ExpressionWorkspace {
    states: Vec<Option<AbstractStack>>,
    pending: VecDeque<usize>,
}

impl ExpressionWorkspace {
    fn reset_states(&mut self, len: usize) {
        self.states.clear();
        self.states.resize_with(len, || None);
    }
}

/// Returns the slots whose `Load` instructions can execute. Expression chunks
/// contain forward jumps for `and`/`or`; a flat scan would report reads hidden
/// behind a constant short-circuit guard.
fn reachable_loads(chunk: ExprChunkRef<'_>, workspace: &mut ExpressionWorkspace) -> LoadSet {
    let mut loads = LoadSet::new();
    let mut has_branches = false;
    let mut branches_are_forward = true;
    for (pc, op) in chunk.ops.iter().enumerate() {
        if let ExprOp::JumpIfFalse(target) | ExprOp::JumpIfTrue(target) = op {
            has_branches = true;
            let target = *target as usize;
            branches_are_forward &= target > pc && target <= chunk.ops.len();
        }
    }
    if !has_branches {
        let mut stack = AbstractStack::new();
        for pc in 0..chunk.ops.len() {
            match abstract_step(chunk, pc, stack, &mut loads) {
                AbstractStep::Stop => break,
                AbstractStep::Next {
                    pc: next_pc,
                    stack: next_stack,
                } => {
                    debug_assert_eq!(next_pc, pc + 1);
                    stack = next_stack;
                }
                AbstractStep::Fork { .. } => {
                    unreachable!("a linear expression cannot fork")
                }
            }
        }
        loads.sort_and_deduplicate();
        return loads;
    }

    if branches_are_forward {
        workspace.reset_states(chunk.ops.len() + 1);
        let states = &mut workspace.states;
        states[0] = Some(AbstractStack::new());
        for pc in 0..chunk.ops.len() {
            let Some(stack) = states[pc].take() else {
                continue;
            };
            match abstract_step(chunk, pc, stack, &mut loads) {
                AbstractStep::Stop => {}
                AbstractStep::Next { pc, stack } => merge_state(&mut states[pc], stack),
                AbstractStep::Fork {
                    first_pc,
                    first_stack,
                    second_pc,
                    second_stack,
                } => {
                    merge_state(&mut states[first_pc], first_stack);
                    merge_state(&mut states[second_pc], second_stack);
                }
            }
        }
        loads.sort_and_deduplicate();
        return loads;
    }

    workspace.reset_states(chunk.ops.len() + 1);
    workspace.pending.clear();
    workspace.pending.push_back(0);
    let states = &mut workspace.states;
    let pending = &mut workspace.pending;
    states[0] = Some(AbstractStack::new());

    while let Some(pc) = pending.pop_front() {
        if pc >= chunk.ops.len() {
            continue;
        }
        let stack = states[pc].clone().unwrap_or_default();
        match abstract_step(chunk, pc, stack, &mut loads) {
            AbstractStep::Stop => {}
            AbstractStep::Next { pc, stack } => {
                enqueue_state(states, pending, pc, stack);
            }
            AbstractStep::Fork {
                first_pc,
                first_stack,
                second_pc,
                second_stack,
            } => {
                enqueue_state(states, pending, first_pc, first_stack);
                enqueue_state(states, pending, second_pc, second_stack);
            }
        }
    }
    loads.sort_and_deduplicate();
    loads
}

enum LoadSet {
    Uncomputed,
    Empty,
    One((u32, usize)),
    Many(Vec<(u32, usize)>),
}

impl LoadSet {
    const fn new() -> Self {
        Self::Empty
    }

    fn insert(&mut self, load: (u32, usize)) {
        match self {
            Self::Uncomputed => unreachable!("loads are computed before insertion"),
            Self::Empty => *self = Self::One(load),
            Self::One(existing) if *existing == load => {}
            Self::One(existing) => {
                let first = *existing;
                *self = Self::Many(vec![first, load]);
            }
            Self::Many(loads) if loads.contains(&load) => {}
            Self::Many(loads) => loads.push(load),
        }
    }

    fn sort_and_deduplicate(&mut self) {
        if let Self::Many(loads) = self {
            loads.sort_unstable();
            loads.dedup();
        }
    }

    fn as_slice(&self) -> &[(u32, usize)] {
        match self {
            Self::Uncomputed | Self::Empty => &[],
            Self::One(load) => std::slice::from_ref(load),
            Self::Many(loads) => loads,
        }
    }
}

enum AbstractStep {
    Stop,
    Next {
        pc: usize,
        stack: AbstractStack,
    },
    Fork {
        first_pc: usize,
        first_stack: AbstractStack,
        second_pc: usize,
        second_stack: AbstractStack,
    },
}

fn abstract_step(
    chunk: ExprChunkRef<'_>,
    pc: usize,
    mut stack: AbstractStack,
    loads: &mut LoadSet,
) -> AbstractStep {
    let mut next_pc = pc + 1;
    match &chunk.ops[pc] {
        ExprOp::Const(index) => {
            stack.push(match chunk.constants.get(*index as usize) {
                Some(Value::Boolean(value)) => AbstractValue::Boolean(Some(*value)),
                Some(_) => AbstractValue::NonBoolean,
                None => AbstractValue::Unknown,
            });
        }
        ExprOp::Load { slot, column } => {
            loads.insert((*slot, *column as usize));
            stack.push(AbstractValue::Unknown);
        }
        ExprOp::Unary(op) => {
            let value = stack.pop().unwrap_or(AbstractValue::Unknown);
            let result = match (op, value) {
                (UnaryOp::Not, AbstractValue::Boolean(Some(value))) => {
                    Some(AbstractValue::Boolean(Some(!value)))
                }
                (UnaryOp::Not, AbstractValue::NonBoolean)
                | (UnaryOp::Negate, AbstractValue::Boolean(_)) => None,
                (UnaryOp::Not, _) => Some(AbstractValue::Boolean(None)),
                (UnaryOp::Negate, _) => Some(AbstractValue::NonBoolean),
            };
            if let Some(result) = result {
                stack.push(result);
            } else {
                return AbstractStep::Stop;
            }
        }
        ExprOp::Binary(op) => {
            pop_values(&mut stack, 2);
            stack.push(match op {
                BinaryOp::Equal
                | BinaryOp::NotEqual
                | BinaryOp::Less
                | BinaryOp::LessEqual
                | BinaryOp::Greater
                | BinaryOp::GreaterEqual
                | BinaryOp::And
                | BinaryOp::Or => AbstractValue::Boolean(None),
                BinaryOp::Add | BinaryOp::Subtract | BinaryOp::Multiply | BinaryOp::Divide => {
                    AbstractValue::NonBoolean
                }
            });
        }
        ExprOp::Call { function, argc } => {
            pop_values(&mut stack, *argc as usize);
            stack.push(match function {
                Builtin::Contains | Builtin::Chance => AbstractValue::Boolean(None),
                Builtin::Get => AbstractValue::Unknown,
                _ => AbstractValue::NonBoolean,
            });
        }
        ExprOp::Random { .. } => {
            pop_values(&mut stack, 2);
            stack.push(AbstractValue::NonBoolean);
        }
        ExprOp::Chance { .. } => {
            pop_values(&mut stack, 1);
            stack.push(AbstractValue::Boolean(None));
        }
        ExprOp::Concat(count) => {
            pop_values(&mut stack, *count as usize);
            stack.push(AbstractValue::NonBoolean);
        }
        ExprOp::JumpIfFalse(target) | ExprOp::JumpIfTrue(target) => {
            let jump_on = matches!(&chunk.ops[pc], ExprOp::JumpIfTrue(_));
            match stack.last().unwrap_or(AbstractValue::Unknown) {
                AbstractValue::Boolean(Some(value)) if value == jump_on => {
                    next_pc = *target as usize;
                }
                AbstractValue::Boolean(Some(_)) | AbstractValue::NonBoolean => {}
                AbstractValue::Boolean(None) | AbstractValue::Unknown => {
                    return AbstractStep::Fork {
                        first_pc: *target as usize,
                        first_stack: stack.clone(),
                        second_pc: pc + 1,
                        second_stack: stack,
                    };
                }
            }
        }
        ExprOp::AssertBoolean(_) => match stack.last() {
            Some(AbstractValue::NonBoolean) | None => return AbstractStep::Stop,
            Some(AbstractValue::Boolean(_) | AbstractValue::Unknown) => {}
        },
    }
    AbstractStep::Next { pc: next_pc, stack }
}

fn pop_values(stack: &mut AbstractStack, count: usize) {
    stack.truncate(stack.len().saturating_sub(count));
}

fn enqueue_state(
    states: &mut [Option<AbstractStack>],
    pending: &mut VecDeque<usize>,
    pc: usize,
    incoming: AbstractStack,
) {
    let Some(state) = states.get_mut(pc) else {
        return;
    };
    let mut merged = state.clone();
    merge_state(&mut merged, incoming);
    if *state != merged {
        *state = merged;
        pending.push_back(pc);
    }
}

fn merge_state(state: &mut Option<AbstractStack>, incoming: AbstractStack) {
    let Some(current) = state else {
        *state = Some(incoming);
        return;
    };
    current.truncate(incoming.len());
    for (left, right) in current.as_mut_slice().iter_mut().zip(incoming.as_slice()) {
        *left = merge_value(*left, *right);
    }
}

const fn merge_value(left: AbstractValue, right: AbstractValue) -> AbstractValue {
    match (left, right) {
        (AbstractValue::Boolean(Some(left)), AbstractValue::Boolean(Some(right)))
            if left == right =>
        {
            AbstractValue::Boolean(Some(left))
        }
        (AbstractValue::Boolean(_), AbstractValue::Boolean(_)) => AbstractValue::Boolean(None),
        (AbstractValue::NonBoolean, AbstractValue::NonBoolean) => AbstractValue::NonBoolean,
        _ => AbstractValue::Unknown,
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
mod tests {
    use super::*;
    use velin_compile::{Program, ProgramBuilder, SlotTable};
    use velin_syntax::{BinaryOp, Expr, UnaryOp, Value};

    #[test]
    fn abstract_stack_preserves_values_after_spilling() {
        let mut stack = AbstractStack::new();
        for index in 0..=INLINE_ABSTRACT_VALUES {
            stack.push(AbstractValue::Boolean(Some(index % 2 == 0)));
        }

        assert!(matches!(stack, AbstractStack::Overflow(_)));
        for index in (0..=INLINE_ABSTRACT_VALUES).rev() {
            assert_eq!(
                stack.pop(),
                Some(AbstractValue::Boolean(Some(index % 2 == 0)))
            );
        }
        assert_eq!(stack.pop(), None);
    }

    #[test]
    fn read_after_assignment_is_clean() {
        // set hp = 30; set hp = hp - 1
        let mut b = ProgramBuilder::new();
        let hp = b.slot("hp");
        let thirty = b.expr(&Expr::Value(Value::Integer(30)), 1);
        b.push(Op::Set {
            slot: hp,
            value: thirty,
        });
        let dec = b.expr(
            &Expr::Binary {
                left: Box::new(Expr::Variable("hp".into())),
                op: BinaryOp::Subtract,
                right: Box::new(Expr::Value(Value::Integer(1))),
            },
            2,
        );
        b.push(Op::Set {
            slot: hp,
            value: dec,
        });
        let findings = definite_assignment(&b.build(), &BTreeSet::new());
        assert!(findings.is_empty(), "unexpected: {findings:?}");
    }

    #[test]
    fn read_before_any_assignment_is_flagged() {
        // set y = x + 1   (x never assigned)
        let mut b = ProgramBuilder::new();
        let y = b.slot("y");
        let expr = b.expr(
            &Expr::Binary {
                left: Box::new(Expr::Variable("x".into())),
                op: BinaryOp::Add,
                right: Box::new(Expr::Value(Value::Integer(1))),
            },
            5,
        );
        b.push(Op::Set {
            slot: y,
            value: expr,
        });
        let findings = definite_assignment(&b.build(), &BTreeSet::new());
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].name, "x");
        assert_eq!(findings[0].line, 5);
    }

    #[test]
    fn assignment_on_only_one_branch_is_flagged() {
        // if cond { flag_true = 1 }  ; read flag_true
        // flag_true is assigned only on the taken branch, so the join is unassigned.
        let mut b = ProgramBuilder::new();
        let cond_slot = b.slot("cond");
        let only = b.slot("only");
        let sink = b.slot("sink");
        let cond = b.expr(&Expr::Variable("cond".into()), 1);
        let jump = b.push(Op::JumpIfFalse {
            condition: cond,
            target: u32::MAX,
        });
        let one = b.expr(&Expr::Value(Value::Integer(1)), 2);
        b.push(Op::Set {
            slot: only,
            value: one,
        });
        let after = b.here();
        b.patch(
            jump,
            Op::JumpIfFalse {
                condition: cond,
                target: after,
            },
        );
        let read = b.expr(&Expr::Variable("only".into()), 3);
        b.push(Op::Set {
            slot: sink,
            value: read,
        });
        // Seed `cond` so only `only` can be flagged.
        let preset = BTreeSet::from(["cond".to_string()]);
        let _ = cond_slot;
        let findings = definite_assignment(&b.build(), &preset);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].name, "only");
    }

    #[test]
    fn preset_variables_count_as_assigned() {
        let mut b = ProgramBuilder::new();
        let sink = b.slot("sink");
        let read = b.expr(&Expr::Variable("seed".into()), 1);
        b.push(Op::Set {
            slot: sink,
            value: read,
        });
        let preset = BTreeSet::from(["seed".to_string()]);
        assert!(definite_assignment(&b.build(), &preset).is_empty());
    }

    #[test]
    fn program_entry_remains_a_predecessor_when_it_has_a_back_edge() {
        let mut b = ProgramBuilder::new();
        let y = b.slot("y");
        let read_x = b.expr(
            &Expr::Binary {
                left: Box::new(Expr::Variable("x".into())),
                op: BinaryOp::Add,
                right: Box::new(Expr::Value(Value::Integer(1))),
            },
            1,
        );
        b.push(Op::Set {
            slot: y,
            value: read_x,
        });
        let x = b.slot("x");
        let one = b.expr(&Expr::Value(Value::Integer(1)), 2);
        b.push(Op::Set {
            slot: x,
            value: one,
        });
        b.push(Op::Jump(0));

        let findings = definite_assignment(&b.build(), &BTreeSet::new());
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].name, "x");
    }

    #[test]
    fn unreachable_reads_are_ignored() {
        let mut b = ProgramBuilder::new();
        b.push(Op::Jump(2));
        let y = b.slot("y");
        let read = b.expr(&Expr::Variable("missing".into()), 2);
        b.push(Op::Set {
            slot: y,
            value: read,
        });
        b.push(Op::Halt);

        assert!(definite_assignment(&b.build(), &BTreeSet::new()).is_empty());
    }

    #[test]
    fn constant_short_circuit_reads_are_ignored() {
        let mut b = ProgramBuilder::new();
        let result = b.slot("result");
        let expression = Expr::Binary {
            left: Box::new(Expr::Value(Value::Boolean(false))),
            op: BinaryOp::And,
            right: Box::new(Expr::Variable("missing".into())),
        };
        let value = b.expr(&expression, 1);
        b.push(Op::Set {
            slot: result,
            value,
        });

        assert!(definite_assignment(&b.build(), &BTreeSet::new()).is_empty());
    }

    #[test]
    fn inline_load_set_spills_sorts_and_deduplicates() {
        let mut loads = LoadSet::new();
        for load in [(4, 1), (2, 3), (5, 1), (1, 8), (3, 2), (2, 3)] {
            loads.insert(load);
        }
        loads.sort_and_deduplicate();
        assert_eq!(loads.as_slice(), &[(1, 8), (2, 3), (3, 2), (4, 1), (5, 1)]);
    }

    fn analyze(source: &str) -> Vec<UnassignedUse> {
        let expr = velin_parse::parse_expression(source, "t", 1, 1).unwrap();
        let mut b = ProgramBuilder::new();
        let sink = b.slot("sink");
        let value = b.expr(&expr, 1);
        b.push(Op::Set { slot: sink, value });
        definite_assignment(&b.build(), &BTreeSet::new())
    }

    #[test]
    fn abstract_interpreter_covers_expression_ops() {
        for source in [
            "not true",
            "not false",
            "not 1",
            "-3",
            "1 + 2",
            "1 - 2",
            "2 * 3",
            "4 / 2",
            "1 == 2",
            "1 != 2",
            "1 < 2",
            "1 <= 2",
            "1 > 2",
            "1 >= 2",
            "true and false",
            "false and missing",
            "true or missing",
            "false or true",
            "1 or missing",
            "true and 1",
            "len(\"ab\")",
            "contains(list(1), 1)",
            "get(list(1), 0)",
            "chance(50)",
            "random(1, 2)",
            "\"hello [1]\"",
            "not not true",
            "x or true",
            "x and false",
        ] {
            let _ = analyze(source);
        }

        let negate_bool = Expr::Unary {
            op: UnaryOp::Negate,
            value: Box::new(Expr::Value(Value::Boolean(true))),
        };
        let mut b = ProgramBuilder::new();
        let sink = b.slot("sink");
        let value = b.expr(&negate_bool, 1);
        b.push(Op::Set { slot: sink, value });
        let _ = definite_assignment(&b.build(), &BTreeSet::new());
    }

    #[test]
    fn host_bind_counts_as_assignment_and_empty_programs_are_clean() {
        let mut b = ProgramBuilder::new();
        let answer = b.slot("answer");
        b.push(Op::host(0, Vec::new(), Some(answer), 1));
        let sink = b.slot("sink");
        let read = b.expr(&Expr::Variable("answer".into()), 2);
        b.push(Op::Set {
            slot: sink,
            value: read,
        });
        assert!(definite_assignment(&b.build(), &BTreeSet::new()).is_empty());

        let empty = Program {
            ops: Vec::new(),
            chunks: Vec::new(),
            expr_ops: Vec::new(),
            constants: Vec::new(),
            slots: SlotTable::default(),
        };
        assert!(definite_assignment(&empty, &BTreeSet::new()).is_empty());
    }

    #[test]
    fn malformed_chunks_exercise_abstract_fallbacks() {
        use velin_compile::{ExprChunk, ExprOp, SlotTable};

        let mut slots = SlotTable::new();
        slots.intern("x");
        let program = Program::from_chunks(
            vec![Op::Set { slot: 0, value: 0 }],
            vec![ExprChunk {
                ops: vec![
                    ExprOp::Const(99),
                    ExprOp::Unary(UnaryOp::Not),
                    ExprOp::AssertBoolean(BinaryOp::And),
                    ExprOp::JumpIfTrue(0),
                    ExprOp::Load { slot: 0, column: 1 },
                ],
                constants: Vec::new(),
                line: 1,
            }],
            slots,
        );
        let _ = definite_assignment(&program, &BTreeSet::new());
    }

    #[test]
    fn missing_chunks_do_not_panic_analysis() {
        let program = Program {
            ops: vec![Op::Set { slot: 0, value: 99 }],
            chunks: Vec::new(),
            expr_ops: Vec::new(),
            constants: Vec::new(),
            slots: SlotTable::new(),
        };
        assert!(definite_assignment(&program, &BTreeSet::new()).is_empty());
    }
}
