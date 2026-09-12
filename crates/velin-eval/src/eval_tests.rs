use super::*;

fn eval(expression: &Expr) -> Result<Value, EvalError> {
    evaluate(expression, &Variables::new(), 1)
}

fn binary(left: Value, op: BinaryOp, right: Value) -> Expr {
    Expr::Binary {
        left: Box::new(Expr::Value(left)),
        op,
        right: Box::new(Expr::Value(right)),
    }
}

#[test]
fn arithmetic_comparisons_and_string_concat_evaluate() {
    assert_eq!(
        eval(&binary(Value::Integer(1), BinaryOp::Add, Value::Integer(2))).unwrap(),
        Value::Integer(3)
    );
    assert_eq!(
        eval(&binary(
            Value::String("a".into()),
            BinaryOp::Add,
            Value::String("b".into())
        ))
        .unwrap(),
        Value::String("ab".into())
    );
    assert_eq!(
        eval(&binary(
            Value::Integer(3),
            BinaryOp::GreaterEqual,
            Value::Integer(3)
        ))
        .unwrap(),
        Value::Boolean(true)
    );
    assert!(
        eval(&binary(
            Value::Integer(1),
            BinaryOp::Divide,
            Value::Integer(0)
        ))
        .unwrap_err()
        .to_string()
        .contains("division by zero")
    );
    assert!(
        eval(&binary(
            Value::Integer(i64::MAX),
            BinaryOp::Add,
            Value::Integer(1)
        ))
        .unwrap_err()
        .to_string()
        .contains("overflow")
    );
}

#[test]
fn boolean_ops_short_circuit_missing_variables() {
    let and = Expr::Binary {
        left: Box::new(Expr::Value(Value::Boolean(false))),
        op: BinaryOp::And,
        right: Box::new(Expr::Variable("missing".into())),
    };
    assert_eq!(eval(&and).unwrap(), Value::Boolean(false));
    let or = Expr::Binary {
        left: Box::new(Expr::Value(Value::Boolean(true))),
        op: BinaryOp::Or,
        right: Box::new(Expr::Variable("missing".into())),
    };
    assert_eq!(eval(&or).unwrap(), Value::Boolean(true));
    assert!(
        eval(&Expr::Unary {
            op: UnaryOp::Not,
            value: Box::new(Expr::Value(Value::Integer(1))),
        })
        .unwrap_err()
        .to_string()
        .contains("boolean")
    );
}

#[test]
fn unbound_variable_reports_its_name() {
    let error = eval(&Expr::Variable("hp".into())).unwrap_err();
    assert!(error.message.contains("`hp`"));
}

#[test]
fn interpolation_renders_holes_deterministically() {
    // "hp is [hp]!" with hp = 7  ->  "hp is 7!"
    let expr = Expr::Interpolate {
        parts: vec![
            StrPart::Literal("hp is ".into()),
            StrPart::Hole(Box::new(Expr::Variable("hp".into()))),
            StrPart::Literal("!".into()),
        ],
    };
    let vars = Variables::from([("hp".into(), Value::Integer(7))]);
    assert_eq!(
        evaluate(&expr, &vars, 1).unwrap(),
        Value::String("hp is 7!".into())
    );
}

#[test]
fn interpolation_propagates_hole_errors() {
    let expr = Expr::Interpolate {
        parts: vec![StrPart::Hole(Box::new(Expr::Variable("missing".into())))],
    };
    assert!(eval(&expr).unwrap_err().message.contains("`missing`"));
}

#[test]
fn string_growth_is_rejected_before_concatenation() {
    let half = "x".repeat(MAX_DATA_TEXT_BYTES / 2 + 1);
    let error = apply_binary(
        Value::String(half.clone().into()),
        BinaryOp::Add,
        Value::String(half.into()),
        1,
    )
    .unwrap_err();
    assert!(error.message.contains("exceeds 1 MiB"));
}

#[test]
fn unary_binary_and_rng_error_paths() {
    assert!(apply_unary(UnaryOp::Negate, Value::Boolean(true), 1).is_err());
    assert!(apply_unary(UnaryOp::Negate, Value::Integer(i64::MIN), 1).is_err());
    assert_eq!(
        apply_unary(UnaryOp::Not, Value::Boolean(true), 1).unwrap(),
        Value::Boolean(false)
    );
    assert!(
        apply_binary(
            Value::Integer(1),
            BinaryOp::Subtract,
            Value::Boolean(true),
            1
        )
        .is_err()
    );
    assert!(
        apply_binary(
            Value::Integer(i64::MIN),
            BinaryOp::Divide,
            Value::Integer(-1),
            1
        )
        .is_err()
    );
    assert!(
        apply_binary(
            Value::Integer(i64::MAX),
            BinaryOp::Multiply,
            Value::Integer(3),
            1
        )
        .is_err()
    );
    assert_eq!(
        apply_binary(Value::Integer(1), BinaryOp::NotEqual, Value::Integer(2), 1).unwrap(),
        Value::Boolean(true)
    );
    assert_eq!(
        apply_binary(
            Value::String("a".into()),
            BinaryOp::Less,
            Value::String("b".into()),
            1
        )
        .unwrap(),
        Value::Boolean(true)
    );
    assert_eq!(
        apply_binary(Value::Integer(3), BinaryOp::Greater, Value::Integer(1), 1).unwrap(),
        Value::Boolean(true)
    );
    assert!(apply_binary(Value::Integer(1), BinaryOp::Less, Value::Boolean(true), 1).is_err());
    assert_eq!(
        apply_binary(Value::Boolean(true), BinaryOp::And, Value::Boolean(true), 1).unwrap(),
        Value::Boolean(true)
    );
    assert_eq!(
        apply_binary(Value::Boolean(false), BinaryOp::Or, Value::Boolean(true), 1).unwrap(),
        Value::Boolean(true)
    );
    assert!(apply_binary(Value::Boolean(true), BinaryOp::And, Value::Integer(1), 1).is_err());

    let random = velin_parse::parse_expression("random(1, 2)", "t", 1, 1).unwrap();
    assert!(
        evaluate(&random, &Variables::new(), 1)
            .unwrap_err()
            .message
            .contains("RNG")
    );
    let mut state = 0;
    assert!(evaluate_with_rng(&random, &Variables::new(), &mut state, 1).is_ok());

    let oversized = Expr::Interpolate {
        parts: vec![
            StrPart::Literal("x".repeat(MAX_DATA_TEXT_BYTES / 2 + 1)),
            StrPart::Literal("y".repeat(MAX_DATA_TEXT_BYTES / 2 + 1)),
        ],
    };
    assert!(
        eval(&oversized)
            .unwrap_err()
            .message
            .contains("exceeds 1 MiB")
    );
}
