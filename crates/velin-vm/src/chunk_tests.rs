use super::*;
use velin_bytecode::SlotTable;
use velin_compile::compile_expression;
use velin_syntax::Expr;

fn run(expr: &Expr, frame: &mut [Option<Value>], slots: &SlotTable) -> Result<Value, EvalError> {
    let mut table = slots.clone();
    let chunk = compile_expression(expr, &mut table, 1);
    eval_chunk(&chunk, frame, |slot| {
        table.name(slot).unwrap_or("?").to_owned()
    })
}

#[test]
fn evaluates_arithmetic_and_builtins() {
    let slots = SlotTable::new();
    let expr = Expr::Binary {
        left: Box::new(Expr::Value(Value::Integer(1))),
        op: BinaryOp::Add,
        right: Box::new(Expr::Binary {
            left: Box::new(Expr::Value(Value::Integer(2))),
            op: BinaryOp::Multiply,
            right: Box::new(Expr::Value(Value::Integer(3))),
        }),
    };
    assert_eq!(run(&expr, &mut [], &slots).unwrap(), Value::Integer(7));
}

#[test]
fn short_circuit_skips_unassigned_right_operand() {
    let mut slots = SlotTable::new();
    slots.intern("a");
    slots.intern("missing");
    let and = Expr::Binary {
        left: Box::new(Expr::Value(Value::Boolean(false))),
        op: BinaryOp::And,
        right: Box::new(Expr::Variable("missing".into())),
    };
    assert_eq!(
        run(&and, &mut [None, None], &slots).unwrap(),
        Value::Boolean(false)
    );
    let or = Expr::Binary {
        left: Box::new(Expr::Value(Value::Boolean(true))),
        op: BinaryOp::Or,
        right: Box::new(Expr::Variable("missing".into())),
    };
    assert_eq!(
        run(&or, &mut [None, None], &slots).unwrap(),
        Value::Boolean(true)
    );
}

#[test]
fn unassigned_slot_names_the_variable() {
    let mut slots = SlotTable::new();
    slots.intern("hp");
    let error = run(&Expr::Variable("hp".into()), &mut [None], &slots).unwrap_err();
    assert!(error.message.contains("`hp`"));
}

#[test]
fn concat_budget_non_boolean_or_and_rng_type() {
    let slots = SlotTable::new();
    let oversized = Expr::Interpolate {
        parts: vec![
            velin_syntax::StrPart::Literal("x".repeat(MAX_DATA_TEXT_BYTES / 2 + 1)),
            velin_syntax::StrPart::Literal("y".repeat(MAX_DATA_TEXT_BYTES / 2 + 1)),
        ],
    };
    assert!(
        run(&oversized, &mut [], &slots)
            .unwrap_err()
            .message
            .contains("exceeds 1 MiB")
    );

    let or = Expr::Binary {
        left: Box::new(Expr::Value(Value::Integer(1))),
        op: BinaryOp::Or,
        right: Box::new(Expr::Value(Value::Boolean(true))),
    };
    assert!(run(&or, &mut [], &slots).is_err());

    let chance = velin_parse::parse_expression("chance(50)", "t", 1, 1).unwrap();
    let mut table = SlotTable::new();
    let chunk = compile_expression(&chance, &mut table, 1);
    let rng = table.rng_state().unwrap();
    let mut frame = vec![None; table.len()];
    frame[rng as usize] = Some(Value::Boolean(true));
    let error = eval_chunk(&chunk, &mut frame, |slot| {
        table.name(slot).unwrap_or("?").to_owned()
    })
    .unwrap_err();
    assert!(error.message.contains("integer"));
}

#[test]
fn malformed_chunks_return_errors_instead_of_panicking() {
    let chunk = ExprChunk {
        ops: vec![ExprOp::Binary {
            dst: 0,
            left: 1,
            op: BinaryOp::Add,
            right: 2,
        }],
        constants: Vec::new(),
        registers: 1,
        result: 0,
        line: 7,
    };
    let error = eval_chunk(&chunk, &mut [], |_| "?".to_owned()).unwrap_err();
    assert_eq!(error.line, 7);
    assert!(error.message.contains("invalid bytecode"));
}
