use super::ssa::{predecessor_lists, successors};
use super::{IrLoop, IrOp, IrOptimization, IrTerminator, TypedIr, operation_value};
use crate::UpdateOp;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use velin_syntax::Value;

impl TypedIr {
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
