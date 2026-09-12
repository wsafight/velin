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
