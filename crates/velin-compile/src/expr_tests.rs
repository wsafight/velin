use super::*;
use velin_syntax::Value;

#[test]
fn constants_are_deduplicated() {
    let mut slots = SlotTable::new();
    let expr = Expr::Binary {
        left: Box::new(Expr::Binary {
            left: Box::new(Expr::Variable("x".into())),
            op: BinaryOp::Add,
            right: Box::new(Expr::Value(Value::Integer(2))),
        }),
        op: BinaryOp::Add,
        right: Box::new(Expr::Value(Value::Integer(2))),
    };
    let chunk = compile_expression(&expr, &mut slots, 1);
    assert_eq!(chunk.constants.len(), 1);
    assert_eq!(chunk.ops.len(), 5);
}

#[test]
fn folds_successful_constant_subtrees_but_preserves_runtime_errors() {
    let mut slots = SlotTable::new();
    let constant = velin_parse::parse_expression("1 + 2 * 3", "test", 1, 1).unwrap();
    let chunk = compile_expression(&constant, &mut slots, 1);
    assert_eq!(
        chunk.ops,
        vec![ExprOp::Const {
            dst: 0,
            constant: 0
        }]
    );
    assert_eq!(chunk.constants, vec![Value::Integer(7)]);

    let dynamic = velin_parse::parse_expression("x + 2 * 3", "test", 1, 1).unwrap();
    let chunk = compile_expression(&dynamic, &mut slots, 1);
    assert_eq!(chunk.ops.len(), 3);
    assert_eq!(chunk.constants, vec![Value::Integer(6)]);

    let failing = velin_parse::parse_expression("1 / 0", "test", 1, 1).unwrap();
    let chunk = compile_expression(&failing, &mut slots, 1);
    assert!(matches!(
        chunk.ops.last(),
        Some(ExprOp::Binary {
            op: BinaryOp::Divide,
            ..
        })
    ));

    let short = velin_parse::parse_expression("false and missing", "test", 1, 1).unwrap();
    let chunk = compile_expression(&short, &mut slots, 1);
    assert_eq!(chunk.constants, vec![Value::Boolean(false)]);
    assert_eq!(
        chunk.ops,
        vec![ExprOp::Const {
            dst: 0,
            constant: 0
        }]
    );
}

#[test]
fn dynamic_expressions_without_constant_subtrees_skip_the_fold_plan() {
    let guard = velin_parse::parse_expression(
        "hp - 10 > 0 and (level * 2 + 5) <= 100 or defeated == false",
        "test",
        1,
        1,
    )
    .unwrap();
    assert!(fold_plan(&guard, 1).is_empty());
}

#[test]
fn folds_pure_arguments_but_not_random_calls() {
    let mut slots = SlotTable::new();
    let random = velin_parse::parse_expression("random(1 + 2, 6)", "test", 1, 1).unwrap();
    let chunk = compile_expression(&random, &mut slots, 1);
    assert_eq!(chunk.constants, vec![Value::Integer(3), Value::Integer(6)]);
    assert!(matches!(chunk.ops.last(), Some(ExprOp::Random { .. })));
}

#[test]
fn folds_constant_interpolation_without_building_an_ast_copy() {
    let mut slots = SlotTable::new();
    let interpolation = velin_parse::parse_expression("\"answer [1 + 2]\"", "test", 1, 1).unwrap();
    let chunk = compile_expression(&interpolation, &mut slots, 1);
    assert_eq!(
        chunk.ops,
        vec![ExprOp::Const {
            dst: 0,
            constant: 0
        }]
    );
    assert_eq!(chunk.constants, vec![Value::String("answer 3".into())]);
}

#[test]
fn variables_are_interned_into_slots() {
    let mut slots = SlotTable::new();
    let expr = Expr::Binary {
        left: Box::new(Expr::Variable("hp".into())),
        op: BinaryOp::Subtract,
        right: Box::new(Expr::Variable("hp".into())),
    };
    let chunk = compile_expression(&expr, &mut slots, 1);
    assert_eq!(slots.len(), 1); // both loads share one slot
    assert!(matches!(chunk.ops[0], ExprOp::Load { slot: 0, .. }));
    assert!(matches!(chunk.ops[1], ExprOp::Load { slot: 0, .. }));
}

#[test]
fn and_emits_a_forward_guard_over_the_right_operand() {
    let mut slots = SlotTable::new();
    let expr = Expr::Binary {
        left: Box::new(Expr::Variable("a".into())),
        op: BinaryOp::And,
        right: Box::new(Expr::Variable("b".into())),
    };
    let chunk = compile_expression(&expr, &mut slots, 1);
    // Load a, JumpIfFalse END, Load b, Binary And -> END == 4
    match chunk.ops[1] {
        ExprOp::JumpIfFalse { target, .. } => assert_eq!(target as usize, chunk.ops.len()),
        ref other => panic!("expected JumpIfFalse guard, got {other:?}"),
    }
}

#[test]
fn or_emits_a_jump_if_true_guard() {
    let mut slots = SlotTable::new();
    let expr = Expr::Binary {
        left: Box::new(Expr::Variable("a".into())),
        op: BinaryOp::Or,
        right: Box::new(Expr::Variable("b".into())),
    };
    let chunk = compile_expression(&expr, &mut slots, 1);
    match chunk.ops[1] {
        ExprOp::JumpIfTrue { target, .. } => assert_eq!(target as usize, chunk.ops.len()),
        ref other => panic!("expected JumpIfTrue guard, got {other:?}"),
    }
}

#[test]
fn random_builtins_emit_dedicated_ops_with_one_shared_state_slot() {
    let mut slots = SlotTable::new();
    let random = velin_parse::parse_expression("random(1, 6)", "test", 1, 1).unwrap();
    let chance = velin_parse::parse_expression("chance(25)", "test", 2, 1).unwrap();
    let random_chunk = compile_expression(&random, &mut slots, 1);
    let chance_chunk = compile_expression(&chance, &mut slots, 2);
    let state_slot = slots.rng_state().expect("RNG slot allocated");
    assert!(matches!(
        random_chunk.ops.last(),
        Some(ExprOp::Random { state_slot: slot, .. }) if *slot == state_slot
    ));
    assert!(matches!(
        chance_chunk.ops.last(),
        Some(ExprOp::Chance { state_slot: slot, .. }) if *slot == state_slot
    ));
    assert_eq!(
        slots
            .names()
            .iter()
            .filter(|name| name.as_str() == crate::RNG_STATE_SLOT)
            .count(),
        1
    );
}
