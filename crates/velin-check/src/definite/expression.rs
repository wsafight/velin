use super::{BinaryOp, Builtin, ExprChunkRef, ExprOp, Value, VecDeque};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum AbstractValue {
    Boolean(Option<bool>),
    NonBoolean,
    Unknown,
}

#[derive(Default)]
pub(super) struct ExpressionWorkspace {
    states: Vec<Option<Vec<AbstractValue>>>,
    pending: VecDeque<usize>,
    queued: Vec<bool>,
}

impl ExpressionWorkspace {
    fn reset(&mut self, op_count: usize) {
        self.states.clear();
        self.states.resize_with(op_count + 1, || None);
        self.pending.clear();
        self.queued.clear();
        self.queued.resize(op_count + 1, false);
    }
}

/// Returns slot loads reachable through register-level short-circuit branches.
pub(super) fn reachable_loads(
    chunk: ExprChunkRef<'_>,
    workspace: &mut ExpressionWorkspace,
) -> LoadSet {
    let mut loads = LoadSet::new();
    workspace.reset(chunk.ops.len());
    workspace.states[0] = Some(vec![AbstractValue::Unknown; usize::from(chunk.registers)]);
    workspace.pending.push_back(0);
    workspace.queued[0] = true;

    while let Some(pc) = workspace.pending.pop_front() {
        workspace.queued[pc] = false;
        if pc >= chunk.ops.len() {
            continue;
        }
        let state = workspace.states[pc].clone().unwrap_or_default();
        for (next, outgoing) in abstract_step(chunk, pc, state, &mut loads) {
            enqueue_state(
                &mut workspace.states,
                &mut workspace.pending,
                &mut workspace.queued,
                next,
                outgoing,
            );
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

fn abstract_step(
    chunk: ExprChunkRef<'_>,
    pc: usize,
    mut registers: Vec<AbstractValue>,
    loads: &mut LoadSet,
) -> Vec<(usize, Vec<AbstractValue>)> {
    let next = pc + 1;
    match &chunk.ops[pc] {
        ExprOp::Const { dst, constant } => {
            let value = match chunk.constants.get(*constant as usize) {
                Some(Value::Boolean(value)) => AbstractValue::Boolean(Some(*value)),
                Some(_) => AbstractValue::NonBoolean,
                None => AbstractValue::Unknown,
            };
            write_register(&mut registers, *dst, value);
        }
        ExprOp::Load { dst, slot, column } => {
            loads.insert((*slot, *column as usize));
            write_register(&mut registers, *dst, AbstractValue::Unknown);
        }
        ExprOp::Unary { dst, op, source } => {
            let source = read_register(&registers, *source);
            let result = match (op, source) {
                (velin_syntax::UnaryOp::Not, AbstractValue::Boolean(Some(value))) => {
                    Some(AbstractValue::Boolean(Some(!value)))
                }
                (velin_syntax::UnaryOp::Not, AbstractValue::NonBoolean)
                | (velin_syntax::UnaryOp::Negate, AbstractValue::Boolean(_)) => None,
                (velin_syntax::UnaryOp::Not, _) => Some(AbstractValue::Boolean(None)),
                (velin_syntax::UnaryOp::Negate, _) => Some(AbstractValue::NonBoolean),
            };
            let Some(result) = result else {
                return Vec::new();
            };
            write_register(&mut registers, *dst, result);
        }
        ExprOp::Binary { dst, op, .. } => {
            write_register(
                &mut registers,
                *dst,
                match op {
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
                },
            );
        }
        ExprOp::Call { dst, function, .. } => {
            write_register(
                &mut registers,
                *dst,
                match function {
                    Builtin::Contains | Builtin::Chance => AbstractValue::Boolean(None),
                    Builtin::Get => AbstractValue::Unknown,
                    _ => AbstractValue::NonBoolean,
                },
            );
        }
        ExprOp::Random { dst, .. } | ExprOp::Concat { dst, .. } => {
            write_register(&mut registers, *dst, AbstractValue::NonBoolean);
        }
        ExprOp::Chance { dst, .. } => {
            write_register(&mut registers, *dst, AbstractValue::Boolean(None));
        }
        ExprOp::JumpIfFalse { condition, target } | ExprOp::JumpIfTrue { condition, target } => {
            let jump_on = matches!(&chunk.ops[pc], ExprOp::JumpIfTrue { .. });
            return match read_register(&registers, *condition) {
                AbstractValue::Boolean(Some(value)) if value == jump_on => {
                    vec![(*target as usize, registers)]
                }
                AbstractValue::Boolean(Some(_)) | AbstractValue::NonBoolean => {
                    vec![(next, registers)]
                }
                AbstractValue::Boolean(None) | AbstractValue::Unknown => {
                    vec![(*target as usize, registers.clone()), (next, registers)]
                }
            };
        }
    }
    vec![(next, registers)]
}

fn read_register(registers: &[AbstractValue], register: u16) -> AbstractValue {
    registers
        .get(register as usize)
        .copied()
        .unwrap_or(AbstractValue::Unknown)
}

fn write_register(registers: &mut [AbstractValue], register: u16, value: AbstractValue) {
    if let Some(destination) = registers.get_mut(register as usize) {
        *destination = value;
    }
}

fn enqueue_state(
    states: &mut [Option<Vec<AbstractValue>>],
    pending: &mut VecDeque<usize>,
    queued: &mut [bool],
    pc: usize,
    incoming: Vec<AbstractValue>,
) {
    let Some(state) = states.get_mut(pc) else {
        return;
    };
    let changed = merge_state(state, incoming);
    if changed && !queued[pc] {
        queued[pc] = true;
        pending.push_back(pc);
    }
}

fn merge_state(state: &mut Option<Vec<AbstractValue>>, incoming: Vec<AbstractValue>) -> bool {
    let Some(current) = state else {
        *state = Some(incoming);
        return true;
    };
    let mut changed = false;
    for (left, right) in current.iter_mut().zip(incoming) {
        let merged = merge_value(*left, right);
        changed |= *left != merged;
        *left = merged;
    }
    changed
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
