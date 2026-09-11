//! End-to-end checks that the parser and evaluator agree.
//!
//! These exercise the full path a host would use: parse a source string into
//! an [`Expr`], then evaluate it against a variable environment.

use std::sync::Arc;
use velin_eval::{Variables, evaluate};
use velin_parse::parse_expression;
use velin_syntax::Value;

fn run(source: &str, variables: &Variables) -> Value {
    let expr = parse_expression(source, "test", 1, 1).expect("source should parse");
    evaluate(&expr, variables, 1).expect("expression should evaluate")
}

#[test]
fn arithmetic_and_precedence_round_trip() {
    let vars = Variables::new();
    assert_eq!(run("1 + 2 * 3", &vars), Value::Integer(7));
    assert_eq!(run("(1 + 2) * 3", &vars), Value::Integer(9));
    assert_eq!(run("-4 + 10", &vars), Value::Integer(6));
}

#[test]
fn comparisons_and_boolean_logic() {
    let vars = Variables::new();
    assert_eq!(run("3 >= 3 and 2 < 5", &vars), Value::Boolean(true));
    assert_eq!(run("not (1 == 2)", &vars), Value::Boolean(true));
    assert_eq!(run("\"a\" + \"b\" == \"ab\"", &vars), Value::Boolean(true));
}

#[test]
fn variables_and_builtins_compose() {
    let vars = Variables::from([
        ("hp".to_string(), Value::Integer(30)),
        (
            "bag".to_string(),
            Value::List(Arc::new(vec![Value::String("key".into())])),
        ),
    ]);
    assert_eq!(run("hp - 10", &vars), Value::Integer(20));
    assert_eq!(run("len(bag)", &vars), Value::Integer(1));
    assert_eq!(run("contains(bag, \"key\")", &vars), Value::Boolean(true));
    assert_eq!(
        run("get(push(bag, \"map\"), 1)", &vars),
        Value::String("map".into())
    );
}

#[test]
fn evaluation_errors_carry_a_line() {
    let expr = parse_expression("1 / 0", "test", 1, 1).unwrap();
    let error = evaluate(&expr, &Variables::new(), 7).unwrap_err();
    assert_eq!(error.line, 7);
    assert!(error.message.contains("division by zero"));
}
