use super::{BinaryOp, Builtin, ExprChunkRef, ExprOp, UnaryOp, Value, VecDeque};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum AbstractValue {
    Boolean(Option<bool>),
    NonBoolean,
    Unknown,
}

pub(super) const INLINE_ABSTRACT_VALUES: usize = 8;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum AbstractStack {
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
    pub(super) const fn new() -> Self {
        Self::Inline {
            values: [AbstractValue::Unknown; INLINE_ABSTRACT_VALUES],
            len: 0,
        }
    }

    pub(super) fn push(&mut self, value: AbstractValue) {
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

    pub(super) fn pop(&mut self) -> Option<AbstractValue> {
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

    pub(super) fn as_slice(&self) -> &[AbstractValue] {
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
pub(super) struct ExpressionWorkspace {
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
pub(super) fn reachable_loads(
    chunk: ExprChunkRef<'_>,
    workspace: &mut ExpressionWorkspace,
) -> LoadSet {
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

pub(super) enum LoadSet {
    Uncomputed,
    Empty,
    One((u32, usize)),
    Many(Vec<(u32, usize)>),
}

impl LoadSet {
    pub(super) const fn new() -> Self {
        Self::Empty
    }

    pub(super) fn insert(&mut self, load: (u32, usize)) {
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

    pub(super) fn sort_and_deduplicate(&mut self) {
        if let Self::Many(loads) = self {
            loads.sort_unstable();
            loads.dedup();
        }
    }

    pub(super) fn as_slice(&self) -> &[(u32, usize)] {
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
