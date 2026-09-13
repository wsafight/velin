use velin_bytecode::Op;

pub(crate) fn is_straight_line(ops: &[Op]) -> bool {
    ops.iter().enumerate().all(|(pc, op)| match op {
        Op::Jump(_) | Op::JumpIfFalse { .. } | Op::JumpIfIntegerCompare { .. } => false,
        Op::Halt => pc + 1 == ops.len(),
        Op::Set { .. }
        | Op::SetConst { .. }
        | Op::CopySlot { .. }
        | Op::Update { .. }
        | Op::Host(_) => true,
    })
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct BasicBlock {
    pub(crate) start: usize,
    pub(crate) end: usize,
}

pub(crate) struct ControlFlow {
    pub(crate) blocks: Vec<BasicBlock>,
    pub(crate) successors: Vec<Vec<usize>>,
    pub(crate) predecessors: Vec<Vec<usize>>,
    pub(crate) reachable: Vec<bool>,
}

impl ControlFlow {
    pub(crate) fn new(ops: &[Op]) -> Self {
        if ops.is_empty() {
            return Self {
                blocks: Vec::new(),
                successors: Vec::new(),
                predecessors: Vec::new(),
                reachable: Vec::new(),
            };
        }

        let mut leaders = vec![false; ops.len()];
        leaders[0] = true;
        for (pc, op) in ops.iter().enumerate() {
            match op {
                Op::Jump(target)
                | Op::JumpIfFalse { target, .. }
                | Op::JumpIfIntegerCompare { target, .. } => {
                    mark_target(&mut leaders, *target);
                    mark_fallthrough(&mut leaders, pc);
                }
                Op::Halt => mark_fallthrough(&mut leaders, pc),
                Op::Set { .. }
                | Op::SetConst { .. }
                | Op::CopySlot { .. }
                | Op::Update { .. }
                | Op::Host(_) => {}
            }
        }

        let starts: Vec<_> = leaders
            .iter()
            .enumerate()
            .filter_map(|(pc, leader)| leader.then_some(pc))
            .collect();
        let blocks: Vec<_> = starts
            .iter()
            .enumerate()
            .map(|(index, start)| BasicBlock {
                start: *start,
                end: starts.get(index + 1).copied().unwrap_or(ops.len()),
            })
            .collect();

        let mut pc_to_block = vec![0; ops.len()];
        for (block_id, block) in blocks.iter().enumerate() {
            pc_to_block[block.start..block.end].fill(block_id);
        }

        let mut successors = vec![Vec::new(); blocks.len()];
        for (block_id, block) in blocks.iter().enumerate() {
            let pc = block.end - 1;
            match &ops[pc] {
                Op::Jump(target) => push_target(&mut successors[block_id], *target, &pc_to_block),
                Op::JumpIfFalse { target, .. } | Op::JumpIfIntegerCompare { target, .. } => {
                    push_pc(&mut successors[block_id], pc + 1, &pc_to_block);
                    push_target(&mut successors[block_id], *target, &pc_to_block);
                }
                Op::Halt => {}
                Op::Set { .. }
                | Op::SetConst { .. }
                | Op::CopySlot { .. }
                | Op::Update { .. }
                | Op::Host(_) => {
                    push_pc(&mut successors[block_id], pc + 1, &pc_to_block);
                }
            }
        }

        let mut predecessors = vec![Vec::new(); blocks.len()];
        for (block, targets) in successors.iter().enumerate() {
            for target in targets {
                predecessors[*target].push(block);
            }
        }

        let mut reachable = vec![false; blocks.len()];
        let mut pending = vec![0];
        while let Some(block) = pending.pop() {
            if reachable[block] {
                continue;
            }
            reachable[block] = true;
            pending.extend(successors[block].iter().copied());
        }

        Self {
            blocks,
            successors,
            predecessors,
            reachable,
        }
    }
}

fn mark_target(leaders: &mut [bool], target: u32) {
    if let Some(leader) = leaders.get_mut(target as usize) {
        *leader = true;
    }
}

fn mark_fallthrough(leaders: &mut [bool], pc: usize) {
    if let Some(leader) = leaders.get_mut(pc + 1) {
        *leader = true;
    }
}

fn push_target(targets: &mut Vec<usize>, pc: u32, pc_to_block: &[usize]) {
    push_pc(targets, pc as usize, pc_to_block);
}

fn push_pc(targets: &mut Vec<usize>, pc: usize, pc_to_block: &[usize]) {
    let Some(block) = pc_to_block.get(pc).copied() else {
        return;
    };
    if !targets.contains(&block) {
        targets.push(block);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn groups_linear_ops_and_splits_branch_targets() {
        let ops = vec![
            Op::JumpIfFalse {
                condition: 0,
                target: 2,
            },
            Op::Jump(3),
            Op::Halt,
            Op::Halt,
        ];
        let flow = ControlFlow::new(&ops);
        assert_eq!(
            flow.blocks
                .iter()
                .map(|block| (block.start, block.end))
                .collect::<Vec<_>>(),
            vec![(0, 1), (1, 2), (2, 3), (3, 4)]
        );
        assert_eq!(flow.successors[0], vec![1, 2]);
        assert_eq!(flow.successors[1], vec![3]);
        assert!(flow.reachable.iter().all(|reachable| *reachable));
    }

    #[test]
    fn keeps_a_linear_program_in_one_block() {
        let ops = vec![
            Op::Set { slot: 0, value: 0 },
            Op::host(0, Vec::new(), None, 1),
            Op::Halt,
        ];
        let flow = ControlFlow::new(&ops);
        assert_eq!(flow.blocks.len(), 1);
        assert_eq!((flow.blocks[0].start, flow.blocks[0].end), (0, 3));
        assert!(is_straight_line(&ops));
    }

    #[test]
    fn straight_line_rejects_branches_and_early_halts() {
        assert!(!is_straight_line(&[
            Op::Halt,
            Op::host(0, Vec::new(), None, 1),
        ]));
        assert!(!is_straight_line(&[Op::Jump(0)]));
    }
}
