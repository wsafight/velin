use super::{IrBlock, IrOp, IrTerminator, IrType, IrValue};
use crate::{ChunkId, Op, Program};
use std::collections::{BTreeMap, BTreeSet};
use velin_syntax::Value;

pub(super) struct SsaPass {
    pub blocks: Vec<IrBlock>,
    pub out_states: Vec<BTreeMap<u32, IrValue>>,
    pub next_value: u32,
    pub needed_phis: Vec<(u32, u32)>,
}

pub(super) fn block_ranges(program: &Program) -> (Vec<(u32, u32)>, Vec<u32>) {
    let mut boundaries = BTreeSet::from([0u32]);
    for (pc, op) in program.ops.iter().enumerate() {
        let pc = u32::try_from(pc).expect("validated pc fits in u32");
        match op {
            Op::Jump(target)
            | Op::JumpIfFalse { target, .. }
            | Op::JumpIfIntegerCompare { target, .. } => {
                boundaries.insert(*target);
                boundaries.insert(pc.saturating_add(1));
            }
            _ => {}
        }
    }
    let boundaries = boundaries
        .into_iter()
        .filter(|pc| usize::try_from(*pc).is_ok_and(|pc| pc < program.ops.len()))
        .collect::<Vec<_>>();
    let mut block_for_pc = vec![0u32; program.ops.len()];
    for (id, window) in boundaries.windows(2).enumerate() {
        for pc in window[0]..window[1] {
            block_for_pc[pc as usize] = u32::try_from(id).expect("block id fits");
        }
    }
    if let Some(start) = boundaries.last().copied() {
        let block = u32::try_from(boundaries.len() - 1).expect("block id fits");
        for pc in block_for_pc.iter_mut().skip(start as usize) {
            *pc = block;
        }
    }
    let mut ranges = Vec::with_capacity(boundaries.len());
    for (index, start) in boundaries.iter().copied().enumerate() {
        let end = boundaries
            .get(index + 1)
            .copied()
            .unwrap_or_else(|| u32::try_from(program.ops.len()).expect("program size fits"));
        ranges.push((start, end));
    }
    if ranges.is_empty() {
        ranges.push((0, 0));
    }
    (ranges, block_for_pc)
}

pub(super) fn skeleton_terminator(
    program: &Program,
    _start: u32,
    end: u32,
    block_for_pc: &[u32],
) -> IrTerminator {
    let fallthrough = block_for_pc.get(end as usize).copied().unwrap_or(u32::MAX);
    let Some(last) = end
        .checked_sub(1)
        .and_then(|pc| program.ops.get(pc as usize))
    else {
        return IrTerminator::Halt;
    };
    match last {
        Op::Jump(target) => IrTerminator::Jump(
            block_for_pc
                .get(*target as usize)
                .copied()
                .unwrap_or(u32::MAX),
        ),
        Op::JumpIfFalse { target, .. } | Op::JumpIfIntegerCompare { target, .. } => {
            IrTerminator::Branch {
                condition: IrValue(u32::MAX),
                if_true: fallthrough,
                if_false: block_for_pc
                    .get(*target as usize)
                    .copied()
                    .unwrap_or(u32::MAX),
            }
        }
        Op::Halt => IrTerminator::Halt,
        _ if end as usize >= program.ops.len() => IrTerminator::Halt,
        _ => IrTerminator::Fallthrough(fallthrough),
    }
}

pub(super) fn predecessor_lists(terminators: &[IrTerminator]) -> Vec<Vec<u32>> {
    let mut predecessors = vec![Vec::new(); terminators.len()];
    for (source, terminator) in terminators.iter().enumerate() {
        for target in successors(terminator) {
            if let Some(list) = predecessors.get_mut(target as usize) {
                let source = u32::try_from(source).expect("block id fits");
                if !list.contains(&source) {
                    list.push(source);
                }
            }
        }
    }
    predecessors
}

pub(super) fn successors(terminator: &IrTerminator) -> Vec<u32> {
    match terminator {
        IrTerminator::Fallthrough(next) | IrTerminator::Jump(next) => {
            (*next != u32::MAX).then_some(*next).into_iter().collect()
        }
        IrTerminator::Branch {
            if_true, if_false, ..
        } => [*if_true, *if_false]
            .into_iter()
            .filter(|target| *target != u32::MAX)
            .collect(),
        IrTerminator::Halt => Vec::new(),
    }
}

#[allow(clippy::too_many_lines)]
pub(super) fn run_ssa_pass(
    program: &Program,
    ranges: &[(u32, u32)],
    skeletons: &[IrTerminator],
    predecessors: &[Vec<u32>],
    phi_slots: &[BTreeSet<u32>],
) -> SsaPass {
    let mut next_value = 0u32;
    let mut phi_values = vec![BTreeMap::new(); ranges.len()];
    for (block, slots) in phi_slots.iter().enumerate() {
        for slot in slots {
            phi_values[block].insert(*slot, new_value(&mut next_value));
        }
    }
    let phi_count = next_value;
    let mut block_bases = Vec::with_capacity(ranges.len());
    for (start, end) in ranges.iter().copied() {
        block_bases.push(next_value);
        for pc in start..end {
            if let Some(op) = program.ops.get(pc as usize)
                && op_produces_value(op)
            {
                next_value = next_value.saturating_add(1);
            }
        }
    }
    let total_values = next_value;
    let mut out_states = vec![BTreeMap::new(); ranges.len()];
    let mut blocks: Vec<Option<IrBlock>> = vec![None; ranges.len()];
    let mut condition_values = vec![None; ranges.len()];
    let mut pending = std::collections::VecDeque::from([0usize]);
    let mut queued = vec![false; ranges.len()];
    queued[0] = true;
    while let Some(block_id) = pending.pop_front() {
        queued[block_id] = false;
        let first_visit = blocks[block_id].is_none();
        let (start, end) = ranges[block_id];
        let block_id_u32 = u32::try_from(block_id).expect("block id fits");
        let mut state = merge_entry_state(block_id_u32, predecessors, &out_states, &phi_values);
        let mut operations = Vec::new();
        for (slot, value) in &phi_values[block_id] {
            operations.push(IrOp::Phi {
                dst: *value,
                slot: *slot,
                inputs: Vec::new(),
                value_type: IrType::Unknown,
            });
            state.insert(*slot, *value);
        }
        let mut block_next = block_bases[block_id];
        for pc in start..end {
            if let Some(op) = program.ops.get(pc as usize)
                && let Some(condition) =
                    lower_ssa_op(program, op, &mut state, &mut operations, &mut block_next)
            {
                condition_values[block_id] = Some(condition);
            }
        }
        let changed = out_states[block_id] != state;
        out_states[block_id] = state;
        let mut terminator = skeletons[block_id].clone();
        if let IrTerminator::Branch { condition, .. } = &mut terminator {
            *condition = condition_values[block_id].unwrap_or(IrValue(u32::MAX));
        }
        blocks[block_id] = Some(IrBlock {
            id: block_id_u32,
            start_pc: start,
            operations,
            terminator,
            predecessors: predecessors[block_id].clone(),
            loop_depth: 0,
            reachable: false,
        });
        if changed || first_visit {
            for successor in successors(&skeletons[block_id]) {
                let successor = successor as usize;
                if !queued[successor] {
                    queued[successor] = true;
                    pending.push_back(successor);
                }
            }
        }
    }
    // Keep unreachable blocks represented in the IR for stable block IDs.
    for block_id in 0..ranges.len() {
        if blocks[block_id].is_some() {
            continue;
        }
        let (start, end) = ranges[block_id];
        let block_id_u32 = u32::try_from(block_id).expect("block id fits");
        let mut state = BTreeMap::new();
        let mut operations = Vec::new();
        for (slot, value) in &phi_values[block_id] {
            operations.push(IrOp::Phi {
                dst: *value,
                slot: *slot,
                inputs: Vec::new(),
                value_type: IrType::Unknown,
            });
            state.insert(*slot, *value);
        }
        let mut block_next = block_bases[block_id];
        for pc in start..end {
            if let Some(op) = program.ops.get(pc as usize) {
                let _ = lower_ssa_op(program, op, &mut state, &mut operations, &mut block_next);
            }
        }
        blocks[block_id] = Some(IrBlock {
            id: block_id_u32,
            start_pc: start,
            operations,
            terminator: skeletons[block_id].clone(),
            predecessors: predecessors[block_id].clone(),
            loop_depth: 0,
            reachable: false,
        });
    }
    let blocks = blocks.into_iter().map(Option::unwrap).collect();
    next_value = total_values.max(phi_count);

    let mut needed_phis = Vec::new();
    for (block, incoming) in predecessors.iter().enumerate() {
        if incoming.len() < 2 {
            continue;
        }
        let mut slots = BTreeSet::new();
        for predecessor in incoming {
            slots.extend(out_states[*predecessor as usize].keys().copied());
        }
        for slot in slots {
            let first = out_states[incoming[0] as usize].get(&slot).copied();
            let same = incoming
                .iter()
                .all(|predecessor| out_states[*predecessor as usize].get(&slot).copied() == first);
            if !same && !phi_slots[block].contains(&slot) {
                needed_phis.push((u32::try_from(block).expect("block id fits"), slot));
            }
        }
    }
    SsaPass {
        blocks,
        out_states,
        next_value,
        needed_phis,
    }
}

pub(super) fn lower_ssa_op(
    program: &Program,
    op: &Op,
    state: &mut BTreeMap<u32, IrValue>,
    operations: &mut Vec<IrOp>,
    next_value: &mut u32,
) -> Option<IrValue> {
    match op {
        Op::SetConst { slot, value, .. } => {
            let dst = new_value(next_value);
            operations.push(IrOp::Constant {
                dst,
                value: value.clone(),
                value_type: value_type(value),
            });
            operations.push(IrOp::Assign {
                slot: *slot,
                value: dst,
            });
            state.insert(*slot, dst);
            None
        }
        Op::CopySlot { slot, source, .. } => {
            let dst = new_value(next_value);
            operations.push(IrOp::Slot {
                dst,
                slot: *source,
                source: state.get(source).copied(),
                value_type: IrType::Unknown,
            });
            operations.push(IrOp::Assign {
                slot: *slot,
                value: dst,
            });
            state.insert(*slot, dst);
            None
        }
        Op::Set { slot, value } => {
            let dst = new_value(next_value);
            let constant = chunk_constant(program, *value);
            operations.push(IrOp::Evaluate {
                dst,
                chunk: *value,
                value_type: constant.as_ref().map_or(IrType::Unknown, value_type),
                barrier: true,
                constant,
            });
            operations.push(IrOp::Assign {
                slot: *slot,
                value: dst,
            });
            state.insert(*slot, dst);
            None
        }
        Op::Update {
            slot, operation, ..
        } => {
            let dst = new_value(next_value);
            operations.push(IrOp::Update {
                dst,
                slot: *slot,
                source: state.get(slot).copied(),
                operation: *operation,
            });
            state.insert(*slot, dst);
            None
        }
        Op::Host(host) => {
            operations.push(IrOp::Barrier);
            if let Some(slot) = host.bind {
                let dst = new_value(next_value);
                operations.push(IrOp::Unknown { dst });
                operations.push(IrOp::Assign { slot, value: dst });
                state.insert(slot, dst);
            }
            None
        }
        Op::JumpIfFalse { condition, .. } => {
            let dst = new_value(next_value);
            let constant = chunk_constant(program, *condition);
            operations.push(IrOp::Evaluate {
                dst,
                chunk: *condition,
                value_type: constant.as_ref().map_or(IrType::Unknown, value_type),
                barrier: constant.is_none(),
                constant,
            });
            Some(dst)
        }
        Op::JumpIfIntegerCompare { condition, .. } => {
            let dst = new_value(next_value);
            operations.push(IrOp::Evaluate {
                dst,
                chunk: *condition,
                constant: None,
                value_type: IrType::Boolean,
                barrier: true,
            });
            Some(dst)
        }
        Op::Jump(_) | Op::Halt => None,
    }
}

pub(super) fn merge_entry_state(
    block: u32,
    predecessors: &[Vec<u32>],
    out_states: &[BTreeMap<u32, IrValue>],
    phi_values: &[BTreeMap<u32, IrValue>],
) -> BTreeMap<u32, IrValue> {
    let incoming = &predecessors[block as usize];
    if incoming.len() == 1 {
        return out_states[incoming[0] as usize].clone();
    }
    let mut slots = BTreeSet::new();
    for predecessor in incoming {
        slots.extend(out_states[*predecessor as usize].keys().copied());
    }
    let mut state = BTreeMap::new();
    for slot in slots {
        if let Some(phi) = phi_values[block as usize].get(&slot) {
            state.insert(slot, *phi);
        } else {
            let first = out_states[incoming[0] as usize].get(&slot).copied();
            if let Some(first) = first
                && incoming.iter().all(|predecessor| {
                    out_states[*predecessor as usize].get(&slot).copied() == Some(first)
                })
            {
                state.insert(slot, first);
            }
        }
    }
    state
}

pub(super) fn fill_phi_inputs(
    blocks: &mut [IrBlock],
    predecessors: &[Vec<u32>],
    out_states: &[BTreeMap<u32, IrValue>],
) {
    for (block_id, block) in blocks.iter_mut().enumerate() {
        for operation in &mut block.operations {
            let IrOp::Phi { slot, inputs, .. } = operation else {
                continue;
            };
            inputs.clear();
            for predecessor in &predecessors[block_id] {
                if let Some(value) = out_states[*predecessor as usize].get(slot) {
                    inputs.push((*predecessor, *value));
                }
            }
        }
    }
}

fn new_value(next: &mut u32) -> IrValue {
    let value = IrValue(*next);
    *next = next.saturating_add(1);
    value
}

fn op_produces_value(op: &Op) -> bool {
    match op {
        Op::SetConst { .. }
        | Op::CopySlot { .. }
        | Op::Set { .. }
        | Op::Update { .. }
        | Op::JumpIfFalse { .. }
        | Op::JumpIfIntegerCompare { .. } => true,
        Op::Host(host) => host.bind.is_some(),
        Op::Jump(_) | Op::Halt => false,
    }
}

fn chunk_constant(program: &Program, chunk: ChunkId) -> Option<Value> {
    let chunk = program.chunk(chunk)?;
    let [crate::ExprOp::Const { dst, constant }] = chunk.ops else {
        return None;
    };
    if *dst != chunk.result {
        return None;
    }
    chunk.constants.get(*constant as usize).cloned()
}

fn value_type(value: &Value) -> IrType {
    match value {
        Value::Integer(_) => IrType::Integer,
        Value::Boolean(_) => IrType::Boolean,
        Value::String(_) => IrType::String,
        Value::List(_) | Value::Record(_) => IrType::Compound,
    }
}
