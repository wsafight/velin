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
fn empty_programs_have_no_blocks() {
    let flow = ControlFlow::new(&[]);
    assert!(flow.blocks.is_empty());
    assert!(flow.reachable.is_empty());
}

#[test]
fn straight_line_rejects_branches_and_early_halts() {
    assert!(!is_straight_line(&[
        Op::Halt,
        Op::host(0, Vec::new(), None, 1),
    ]));
    assert!(!is_straight_line(&[Op::Jump(0)]));
}
