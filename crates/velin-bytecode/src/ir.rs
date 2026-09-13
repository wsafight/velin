//! Typed basic-block IR used by execution preparation.
//!
//! The IR is deliberately conservative: host effects, random operations and
//! potentially failing operations are barriers. It provides a real SSA view
//! for control-flow joins and performs only transformations whose safety is
//! independent of the canonical bytecode representation.

use crate::{ChunkId, Op, Program, UpdateOp};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use velin_syntax::{BinaryOp, UnaryOp, Value};

/// The runtime type domain carried by an IR value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IrType {
    Unknown,
    Integer,
    Boolean,
    String,
    Compound,
}

/// A monotonic SSA value identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct IrValue(pub u32);

/// One typed operation in a basic block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IrOp {
    /// A merge value with one incoming definition for each predecessor that
    /// assigned the slot. Missing inputs represent an unassigned path.
    Phi {
        dst: IrValue,
        slot: u32,
        inputs: Vec<(u32, IrValue)>,
        value_type: IrType,
    },
    Constant {
        dst: IrValue,
        value: Value,
        value_type: IrType,
    },
    Slot {
        dst: IrValue,
        slot: u32,
        source: Option<IrValue>,
        value_type: IrType,
    },
    /// An expression chunk evaluated by the canonical register VM.
    Evaluate {
        dst: IrValue,
        chunk: ChunkId,
        constant: Option<Value>,
        value_type: IrType,
        barrier: bool,
    },
    Unary {
        dst: IrValue,
        op: UnaryOp,
        source: IrValue,
        value_type: IrType,
    },
    Binary {
        dst: IrValue,
        left: IrValue,
        op: BinaryOp,
        right: IrValue,
        value_type: IrType,
    },
    Update {
        dst: IrValue,
        slot: u32,
        source: Option<IrValue>,
        operation: UpdateOp,
    },
    /// A checked integer induction update recognized inside a loop.
    Induction {
        dst: IrValue,
        slot: u32,
        source: Option<IrValue>,
        step: i64,
    },
    /// A host result or another value whose definition is external to the IR.
    Unknown {
        dst: IrValue,
    },
    Assign {
        slot: u32,
        value: IrValue,
    },
    Barrier,
}

/// Control-flow edge at the end of a basic block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IrTerminator {
    Fallthrough(u32),
    Jump(u32),
    Branch {
        condition: IrValue,
        if_true: u32,
        if_false: u32,
    },
    Halt,
}

/// A natural loop discovered from a back edge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IrLoop {
    pub header: u32,
    pub blocks: Vec<u32>,
    pub back_edges: Vec<(u32, u32)>,
    pub preheader: Option<u32>,
}

/// A safe transformation recorded by the internal optimizer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IrOptimization {
    Constant {
        value: IrValue,
        constant: Value,
    },
    BranchFolded {
        block: u32,
        target: u32,
    },
    Hoisted {
        loop_header: u32,
        from_block: u32,
        value: IrValue,
    },
    StrengthReduced {
        loop_header: u32,
        slot: u32,
        step: i64,
    },
}

/// One basic block with stable source program-counter boundaries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IrBlock {
    pub id: u32,
    pub start_pc: u32,
    pub operations: Vec<IrOp>,
    pub terminator: IrTerminator,
    pub predecessors: Vec<u32>,
    pub loop_depth: u16,
    pub reachable: bool,
}

/// A typed SSA view and conservative optimizer for a validated program.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypedIr {
    blocks: Vec<IrBlock>,
    entry: u32,
    next_value: u32,
    constants: Vec<Option<Value>>,
    loops: Vec<IrLoop>,
    optimizations: Vec<IrOptimization>,
}

#[derive(Debug)]
struct SsaPass {
    blocks: Vec<IrBlock>,
    out_states: Vec<BTreeMap<u32, IrValue>>,
    next_value: u32,
    needed_phis: Vec<(u32, u32)>,
}

impl TypedIr {
    /// Builds basic blocks, SSA definitions, and merge phis from a validated
    /// program.
    #[must_use]
    pub fn from_program(program: &Program) -> Self {
        let (ranges, block_for_pc) = block_ranges(program);
        let skeletons = ranges
            .iter()
            .map(|(start, end)| skeleton_terminator(program, *start, *end, &block_for_pc))
            .collect::<Vec<_>>();
        let predecessors = predecessor_lists(&skeletons);
        let mut phi_slots = vec![BTreeSet::new(); ranges.len()];
        let mut pass = run_ssa_pass(program, &ranges, &skeletons, &predecessors, &phi_slots);
        for _ in 0..ranges.len().saturating_add(1) {
            if pass.needed_phis.is_empty() {
                break;
            }
            let mut changed = false;
            for (block, slot) in pass.needed_phis.drain(..) {
                changed |= phi_slots[block as usize].insert(slot);
            }
            if !changed {
                break;
            }
            pass = run_ssa_pass(program, &ranges, &skeletons, &predecessors, &phi_slots);
        }

        let mut ir = Self {
            blocks: pass.blocks,
            entry: 0,
            next_value: pass.next_value,
            constants: vec![None; pass.next_value as usize],
            loops: Vec::new(),
            optimizations: Vec::new(),
        };
        fill_phi_inputs(&mut ir.blocks, &predecessors, &pass.out_states);
        ir.optimize();
        ir
    }

    /// Runs sparse constant propagation, branch folding, natural-loop
    /// discovery, safe invariant hoisting, and induction-update recognition.
    pub fn optimize(&mut self) {
        self.optimizations.clear();
        self.constants = vec![None; self.next_value as usize];
        self.mark_reachable();
        self.propagate_constants();
        self.fold_constant_branches();
        self.refresh_predecessors();
        self.prune_phi_inputs();
        self.mark_reachable();
        self.discover_loops();
        self.hoist_loop_invariants();
        self.reduce_induction_updates();
    }

    #[must_use]
    pub fn blocks(&self) -> &[IrBlock] {
        &self.blocks
    }

    #[must_use]
    pub const fn entry(&self) -> u32 {
        self.entry
    }

    #[must_use]
    pub const fn value_count(&self) -> u32 {
        self.next_value
    }

    /// Returns the compile-time constant known for an SSA value, if any.
    #[must_use]
    pub fn constant(&self, value: IrValue) -> Option<&Value> {
        self.constants.get(value.0 as usize)?.as_ref()
    }

    #[must_use]
    pub fn loops(&self) -> &[IrLoop] {
        &self.loops
    }

    #[must_use]
    pub fn optimizations(&self) -> &[IrOptimization] {
        &self.optimizations
    }

    fn propagate_constants(&mut self) {
        for _ in 0..self.next_value.saturating_add(1) {
            let mut next = vec![None; self.next_value as usize];
            for block in self.blocks.iter().filter(|block| block.reachable) {
                for op in &block.operations {
                    let (dst, value) = match op {
                        IrOp::Phi { dst, inputs, .. } => {
                            if inputs.len() != block.predecessors.len() {
                                continue;
                            }
                            let mut values = inputs.iter().filter_map(|(_, value)| {
                                self.constants.get(value.0 as usize)?.as_ref()
                            });
                            let Some(first) = values.next() else {
                                continue;
                            };
                            if values.all(|value| value == first) {
                                (*dst, Some(first.clone()))
                            } else {
                                (*dst, None)
                            }
                        }
                        IrOp::Constant { dst, value, .. } => (*dst, Some(value.clone())),
                        IrOp::Slot { dst, source, .. } => (
                            *dst,
                            source
                                .and_then(|source| self.constants.get(source.0 as usize)?.clone()),
                        ),
                        IrOp::Evaluate { dst, constant, .. } => (*dst, constant.clone()),
                        IrOp::Update {
                            dst,
                            source,
                            operation: UpdateOp::AddInteger { value },
                            ..
                        } => {
                            let result = source
                                .and_then(|source| self.constants.get(source.0 as usize)?.as_ref())
                                .and_then(|source| match source {
                                    Value::Integer(source) => {
                                        source.checked_add(*value).map(Value::Integer)
                                    }
                                    _ => None,
                                });
                            (*dst, result)
                        }
                        IrOp::Update { .. }
                        | IrOp::Induction { .. }
                        | IrOp::Unary { .. }
                        | IrOp::Binary { .. }
                        | IrOp::Unknown { .. }
                        | IrOp::Assign { .. }
                        | IrOp::Barrier => continue,
                    };
                    next[dst.0 as usize] = value;
                    if let Some(value) = &next[dst.0 as usize] {
                        self.optimizations.push(IrOptimization::Constant {
                            value: dst,
                            constant: value.clone(),
                        });
                    }
                }
            }
            if next == self.constants {
                break;
            }
            self.constants = next;
        }
        self.optimizations.dedup();
    }

    fn fold_constant_branches(&mut self) {
        let constants = self.constants.clone();
        for block in &mut self.blocks {
            let IrTerminator::Branch {
                condition,
                if_true,
                if_false,
            } = block.terminator
            else {
                continue;
            };
            let Some(Some(Value::Boolean(value))) = constants.get(condition.0 as usize) else {
                continue;
            };
            let target = if *value { if_true } else { if_false };
            block.terminator = IrTerminator::Jump(target);
            self.optimizations.push(IrOptimization::BranchFolded {
                block: block.id,
                target,
            });
        }
    }

    fn mark_reachable(&mut self) {
        for block in &mut self.blocks {
            block.reachable = false;
        }
        let mut pending = VecDeque::from([self.entry]);
        while let Some(id) = pending.pop_front() {
            let Some(block) = self.blocks.get_mut(id as usize) else {
                continue;
            };
            if block.reachable {
                continue;
            }
            block.reachable = true;
            pending.extend(successors(&block.terminator));
        }
    }

    fn refresh_predecessors(&mut self) {
        let terminators = self
            .blocks
            .iter()
            .map(|block| block.terminator.clone())
            .collect::<Vec<_>>();
        let predecessors = predecessor_lists(&terminators);
        for (block, predecessors) in self.blocks.iter_mut().zip(predecessors) {
            block.predecessors = predecessors;
        }
    }

    fn prune_phi_inputs(&mut self) {
        for block in &mut self.blocks {
            let predecessors = block.predecessors.iter().copied().collect::<BTreeSet<_>>();
            for operation in &mut block.operations {
                let IrOp::Phi { inputs, .. } = operation else {
                    continue;
                };
                inputs.retain(|(predecessor, _)| predecessors.contains(predecessor));
            }
        }
    }

    fn discover_loops(&mut self) {
        self.loops.clear();
        for block in &mut self.blocks {
            block.loop_depth = 0;
        }
        let dominators = self.dominators();
        let mut loops = BTreeMap::<u32, IrLoop>::new();
        let block_count = u32::try_from(self.blocks.len()).expect("block count fits");
        for source in 0..block_count {
            for target in successors(&self.blocks[source as usize].terminator) {
                let Some(target_block) = self.blocks.get(target as usize) else {
                    continue;
                };
                if !self.blocks[source as usize].reachable
                    || !target_block.reachable
                    || !dominators[source as usize].contains(&target)
                {
                    continue;
                }
                let mut members = BTreeSet::from([target]);
                let mut pending = Vec::new();
                if source != target {
                    members.insert(source);
                    pending.push(source);
                }
                while let Some(block) = pending.pop() {
                    let Some(current) = self.blocks.get(block as usize) else {
                        continue;
                    };
                    for predecessor in &current.predecessors {
                        if *predecessor >= block_count {
                            continue;
                        }
                        if *predecessor != target && members.insert(*predecessor) {
                            pending.push(*predecessor);
                        }
                    }
                }
                let loop_entry = loops.entry(target).or_insert_with(|| IrLoop {
                    header: target,
                    blocks: Vec::new(),
                    back_edges: Vec::new(),
                    preheader: None,
                });
                let mut all_members = loop_entry.blocks.iter().copied().collect::<BTreeSet<_>>();
                all_members.extend(members);
                loop_entry.blocks = all_members.into_iter().collect();
                if !loop_entry.back_edges.contains(&(source, target)) {
                    loop_entry.back_edges.push((source, target));
                }
            }
        }
        for loop_info in loops.values_mut() {
            let members = loop_info.blocks.iter().copied().collect::<BTreeSet<_>>();
            let outside = self.blocks[loop_info.header as usize]
                .predecessors
                .iter()
                .copied()
                .filter(|predecessor| {
                    self.blocks
                        .get(*predecessor as usize)
                        .is_some_and(|block| block.reachable)
                })
                .filter(|predecessor| !members.contains(predecessor))
                .collect::<Vec<_>>();
            loop_info.preheader = (outside.len() == 1).then(|| outside[0]);
        }
        self.loops = loops.into_values().collect();
        for loop_info in &self.loops {
            for block in &loop_info.blocks {
                if let Some(block) = self.blocks.get_mut(*block as usize) {
                    block.loop_depth = block.loop_depth.saturating_add(1);
                }
            }
        }
    }

    fn hoist_loop_invariants(&mut self) {
        let definitions = self.value_definition_blocks();
        for loop_info in self.loops.clone() {
            let Some(preheader) = loop_info.preheader else {
                continue;
            };
            let members = loop_info.blocks.iter().copied().collect::<BTreeSet<_>>();
            let mut moved = Vec::new();
            for block_id in &loop_info.blocks {
                let block = &mut self.blocks[*block_id as usize];
                let mut retained = Vec::with_capacity(block.operations.len());
                for operation in block.operations.drain(..) {
                    let value = operation_value(&operation);
                    let invariant = matches!(operation, IrOp::Constant { .. })
                        || matches!(
                            &operation,
                            IrOp::Slot {
                                source: Some(source),
                                ..
                            } if definitions
                                .get(source.0 as usize)
                                .and_then(|block| *block)
                                .is_some_and(|block| !members.contains(&block))
                        );
                    if invariant && let Some(value) = value {
                        moved.push((value, *block_id, operation));
                        continue;
                    }
                    retained.push(operation);
                }
                block.operations = retained;
            }
            for (value, from_block, operation) in moved {
                self.blocks[preheader as usize].operations.push(operation);
                self.optimizations.push(IrOptimization::Hoisted {
                    loop_header: loop_info.header,
                    from_block,
                    value,
                });
            }
        }
    }

    fn reduce_induction_updates(&mut self) {
        for loop_info in self.loops.clone() {
            let members = loop_info.blocks.iter().copied().collect::<BTreeSet<_>>();
            for block_id in members {
                for operation in &mut self.blocks[block_id as usize].operations {
                    let Some((dst, slot, source, step)) = (match operation {
                        IrOp::Update {
                            dst,
                            slot,
                            source,
                            operation: UpdateOp::AddInteger { value },
                        } => Some((*dst, *slot, *source, *value)),
                        _ => None,
                    }) else {
                        continue;
                    };
                    *operation = IrOp::Induction {
                        dst,
                        slot,
                        source,
                        step,
                    };
                    self.optimizations.push(IrOptimization::StrengthReduced {
                        loop_header: loop_info.header,
                        slot,
                        step,
                    });
                }
            }
        }
    }

    fn value_definition_blocks(&self) -> Vec<Option<u32>> {
        let mut definitions = vec![None; self.next_value as usize];
        for block in &self.blocks {
            for operation in &block.operations {
                if let Some(value) = operation_value(operation) {
                    definitions[value.0 as usize] = Some(block.id);
                }
            }
        }
        definitions
    }

    fn dominators(&self) -> Vec<BTreeSet<u32>> {
        let block_count = self.blocks.len();
        let reachable = self
            .blocks
            .iter()
            .filter(|block| block.reachable)
            .map(|block| block.id)
            .collect::<BTreeSet<_>>();
        let mut dominators = vec![BTreeSet::new(); block_count];
        for block in &self.blocks {
            if !block.reachable {
                continue;
            }
            if block.id == self.entry {
                dominators[block.id as usize].insert(self.entry);
            } else {
                dominators[block.id as usize].clone_from(&reachable);
            }
        }
        loop {
            let mut changed = false;
            for block in &self.blocks {
                if !block.reachable || block.id == self.entry {
                    continue;
                }
                let mut predecessors = block
                    .predecessors
                    .iter()
                    .copied()
                    .filter(|predecessor| reachable.contains(predecessor));
                let Some(first) = predecessors.next() else {
                    continue;
                };
                let mut intersection = dominators[first as usize].clone();
                for predecessor in predecessors {
                    intersection
                        .retain(|dominator| dominators[predecessor as usize].contains(dominator));
                }
                intersection.insert(block.id);
                if intersection != dominators[block.id as usize] {
                    dominators[block.id as usize] = intersection;
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }
        dominators
    }
}

fn block_ranges(program: &Program) -> (Vec<(u32, u32)>, Vec<u32>) {
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

fn skeleton_terminator(
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

fn predecessor_lists(terminators: &[IrTerminator]) -> Vec<Vec<u32>> {
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

fn successors(terminator: &IrTerminator) -> Vec<u32> {
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

fn run_ssa_pass(
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
    let mut out_states = vec![BTreeMap::new(); ranges.len()];
    let mut blocks = Vec::with_capacity(ranges.len());
    let mut condition_values = vec![None; ranges.len()];
    for (block_id, (start, end)) in ranges.iter().copied().enumerate() {
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
        for pc in start..end {
            if let Some(op) = program.ops.get(pc as usize)
                && let Some(condition) =
                    lower_ssa_op(program, op, &mut state, &mut operations, &mut next_value)
            {
                condition_values[block_id] = Some(condition);
            }
        }
        out_states[block_id] = state;
        let mut terminator = skeletons[block_id].clone();
        if let IrTerminator::Branch { condition, .. } = &mut terminator {
            *condition = condition_values[block_id].unwrap_or(IrValue(u32::MAX));
        }
        blocks.push(IrBlock {
            id: block_id_u32,
            start_pc: start,
            operations,
            terminator,
            predecessors: predecessors[block_id].clone(),
            loop_depth: 0,
            reachable: false,
        });
    }

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

fn lower_ssa_op(
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

fn merge_entry_state(
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

fn fill_phi_inputs(
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

fn operation_value(operation: &IrOp) -> Option<IrValue> {
    match operation {
        IrOp::Phi { dst, .. }
        | IrOp::Constant { dst, .. }
        | IrOp::Slot { dst, .. }
        | IrOp::Evaluate { dst, .. }
        | IrOp::Unary { dst, .. }
        | IrOp::Binary { dst, .. }
        | IrOp::Update { dst, .. }
        | IrOp::Induction { dst, .. }
        | IrOp::Unknown { dst } => Some(*dst),
        IrOp::Assign { .. } | IrOp::Barrier => None,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ExprChunk, ExprOp, SlotTable};

    fn constant_chunk(value: Value) -> ExprChunk {
        ExprChunk {
            ops: vec![ExprOp::Const {
                dst: 0,
                constant: 0,
            }],
            constants: vec![value],
            registers: 1,
            result: 0,
            line: 1,
        }
    }

    #[test]
    fn builds_real_phi_values_at_control_flow_joins() {
        let mut slots = SlotTable::new();
        let value = slots.intern("value");
        let condition = slots.intern("condition");
        let program = Program::from_chunks(
            vec![
                Op::SetConst {
                    slot: condition,
                    value: Value::Boolean(true),
                    line: 1,
                },
                Op::SetConst {
                    slot: value,
                    value: Value::Integer(1),
                    line: 1,
                },
                Op::JumpIfFalse {
                    condition: 0,
                    target: 5,
                },
                Op::SetConst {
                    slot: value,
                    value: Value::Integer(2),
                    line: 2,
                },
                Op::Jump(6),
                Op::SetConst {
                    slot: value,
                    value: Value::Integer(3),
                    line: 3,
                },
                Op::Halt,
            ],
            vec![constant_chunk(Value::Boolean(true))],
            slots,
        );
        let ir = TypedIr::from_program(&program);
        assert!(ir.blocks().iter().any(|block| {
            block.operations.iter().any(|operation| {
                matches!(operation, IrOp::Phi { slot, inputs, .. } if *slot == value && inputs.len() == 2)
            })
        }));
        assert!(
            ir.optimizations()
                .iter()
                .any(|optimization| matches!(optimization, IrOptimization::BranchFolded { .. }))
        );
    }

    #[test]
    fn folds_constants_and_discovers_loop_strength_reduction() {
        let mut slots = SlotTable::new();
        let counter = slots.intern("counter");
        let condition = slots.intern("condition");
        let invariant = slots.intern("invariant");
        let program = Program::from_chunks(
            vec![
                Op::SetConst {
                    slot: counter,
                    value: Value::Integer(0),
                    line: 1,
                },
                Op::JumpIfFalse {
                    condition: 0,
                    target: 5,
                },
                Op::SetConst {
                    slot: invariant,
                    value: Value::Integer(7),
                    line: 2,
                },
                Op::Update {
                    slot: counter,
                    operation: UpdateOp::AddInteger { value: 1 },
                    line: 2,
                    column: 1,
                },
                Op::Jump(1),
                Op::Halt,
            ],
            vec![ExprChunk {
                ops: vec![ExprOp::Load {
                    dst: 0,
                    slot: condition,
                    column: 1,
                }],
                constants: Vec::new(),
                registers: 1,
                result: 0,
                line: 1,
            }],
            slots,
        );
        let ir = TypedIr::from_program(&program);
        assert!(
            ir.optimizations()
                .iter()
                .any(|optimization| matches!(optimization, IrOptimization::Constant { .. }))
        );
        assert!(
            ir.optimizations()
                .iter()
                .any(|optimization| matches!(optimization, IrOptimization::StrengthReduced { .. }))
        );
        assert!(
            ir.optimizations()
                .iter()
                .any(|optimization| matches!(optimization, IrOptimization::Hoisted { .. }))
        );
        assert!(ir.loops().iter().any(|loop_info| loop_info.header == 1));
    }

    #[test]
    fn keeps_reachability_and_ssa_values_bounded_for_empty_programs() {
        let program = Program::from_chunks(Vec::new(), Vec::new(), SlotTable::new());
        let ir = TypedIr::from_program(&program);
        assert_eq!(ir.entry(), 0);
        assert_eq!(ir.blocks().len(), 1);
        assert!(ir.blocks()[0].reachable);
    }

    #[test]
    fn handles_entry_back_edges_without_a_preheader() {
        let program = Program::from_chunks(vec![Op::Jump(0)], Vec::new(), SlotTable::new());
        let ir = TypedIr::from_program(&program);
        let loop_info = ir
            .loops()
            .iter()
            .find(|loop_info| loop_info.header == 0)
            .expect("entry back edge is a natural loop");
        assert_eq!(loop_info.preheader, None);
    }

    #[test]
    fn keeps_a_self_loop_preheader_outside_the_loop() {
        let mut slots = SlotTable::new();
        let slot = slots.intern("slot");
        let program = Program::from_chunks(
            vec![
                Op::SetConst {
                    slot,
                    value: Value::Integer(1),
                    line: 1,
                },
                Op::Jump(1),
            ],
            Vec::new(),
            slots,
        );
        let ir = TypedIr::from_program(&program);
        let loop_info = ir
            .loops()
            .iter()
            .find(|loop_info| loop_info.header == 1)
            .expect("self edge is a natural loop");
        assert_eq!(loop_info.blocks, vec![1]);
        assert_eq!(loop_info.preheader, Some(0));
    }
}
