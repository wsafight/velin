use super::*;
use velin_parse::parse_expression;

fn errors_for(source: &str, env: &Environment) -> Vec<String> {
    let expr = parse_expression(source, "t", 1, 1).unwrap();
    let mut errors = Vec::new();
    infer(&expr, env, &mut errors);
    errors.into_iter().map(|error| error.message).collect()
}

#[test]
fn provable_mismatches_are_reported() {
    let env = Environment::new();
    assert!(!errors_for("\"a\" + 1", &env).is_empty());
    assert!(!errors_for("not 3", &env).is_empty());
    assert!(!errors_for("1 - \"x\"", &env).is_empty());
    assert!(!errors_for("get(list(1), \"key\")", &env).is_empty());
    assert!(!errors_for("push(record(\"a\", 1), 2)", &env).is_empty());
}

#[test]
fn valid_expressions_have_no_errors() {
    let env = Environment::from([("hp".into(), Type::Integer)]);
    assert!(errors_for("hp + 1", &env).is_empty());
    assert!(errors_for("\"a\" + \"b\"", &env).is_empty());
    assert!(errors_for("len(list(1, 2))", &env).is_empty());
    assert!(errors_for("len(list(1, 2, 3, 4, 5))", &env).is_empty());
    assert!(errors_for("get(list(1, 2), 0)", &env).is_empty());
    assert!(errors_for("contains(\"abc\", \"b\")", &env).is_empty());
    assert!(errors_for("random(1, 6)", &env).is_empty());
    assert!(errors_for("chance(50)", &env).is_empty());
    assert!(!errors_for("random(\"low\", 6)", &env).is_empty());
    assert!(!errors_for("chance(false)", &env).is_empty());
}

#[test]
fn constant_short_circuit_skips_unreachable_type_errors() {
    let env = Environment::new();
    assert!(errors_for("false and (1 + \"x\")", &env).is_empty());
    assert!(errors_for("true or (1 + \"x\")", &env).is_empty());
    assert!(!errors_for("flag and (1 + \"x\")", &env).is_empty());
}

#[test]
fn contains_checks_record_and_string_keys() {
    let env = Environment::new();
    assert!(!errors_for("contains(\"abc\", 1)", &env).is_empty());
    assert!(!errors_for("contains(record(\"a\", 1), 1)", &env).is_empty());
    assert!(errors_for("contains(list(1), 1)", &env).is_empty());
}

#[test]
fn unknown_variables_never_false_positive() {
    let env = Environment::new(); // everything is Unknown
    assert!(errors_for("mystery + 1", &env).is_empty());
    assert!(errors_for("if_flag and other", &env).is_empty());
    assert!(errors_for("get(mystery, key)", &env).is_empty());
}

#[test]
fn every_type_and_builtin_path_is_reported() {
    use std::collections::BTreeMap;
    use std::sync::Arc;
    use velin_syntax::{Expr, Value};

    assert_eq!(Type::Unknown.name(), "unknown");
    assert_eq!(Type::List.name(), "list");
    assert_eq!(Type::Record.name(), "record");
    assert_eq!(Type::from(&Value::List(Arc::new(Vec::new()))), Type::List);
    assert_eq!(
        Type::from(&Value::Record(Arc::new(BTreeMap::new()))),
        Type::Record
    );

    let env = Environment::new();
    assert!(!errors_for("- \"x\"", &env).is_empty());
    assert!(!errors_for("true < 1", &env).is_empty());
    assert!(!errors_for("\"a\" >= 3", &env).is_empty());
    assert!(!errors_for("len(1)", &env).is_empty());
    assert!(!errors_for("contains(1, 2)", &env).is_empty());
    assert!(!errors_for("record(1, 2)", &env).is_empty());
    assert!(!errors_for("put(list(1), \"k\", 2)", &env).is_empty());
    assert!(!errors_for("remove(list(1), \"k\")", &env).is_empty());
    assert!(errors_for("put(list(1), 0, 2)", &env).is_empty());
    assert!(errors_for("remove(record(\"a\", 1), \"a\")", &env).is_empty());
    assert!(errors_for("len(\"ab\")", &env).is_empty());
    assert!(errors_for("len(record(\"a\", 1))", &env).is_empty());
    assert!(errors_for("contains(record(\"a\", 1), \"a\")", &env).is_empty());
    assert!(
        errors_for("\"hello [1 - true]\"", &env)
            .iter()
            .any(|message| message.contains("arithmetic"))
    );
    assert!(errors_for("\"[hp]\"", &env).is_empty());

    let mut errors = Vec::new();
    let too_many_args = Expr::Invoke {
        function: velin_syntax::Builtin::Len,
        arguments: vec![
            Expr::Value(Value::Integer(1)),
            Expr::Value(Value::Integer(2)),
        ],
    };
    infer(&too_many_args, &env, &mut errors);
    assert!(
        errors
            .iter()
            .any(|error| error.message.contains("does not accept"))
    );

    let mut errors = Vec::new();
    let empty_get = Expr::Invoke {
        function: velin_syntax::Builtin::Get,
        arguments: Vec::new(),
    };
    assert_eq!(infer(&empty_get, &env, &mut errors), Type::Unknown);

    let mut errors = Vec::new();
    let empty_put = Expr::Invoke {
        function: velin_syntax::Builtin::Put,
        arguments: Vec::new(),
    };
    assert_eq!(infer(&empty_put, &env, &mut errors), Type::Unknown);

    let mut errors = Vec::new();
    let empty_remove = Expr::Invoke {
        function: velin_syntax::Builtin::Remove,
        arguments: Vec::new(),
    };
    assert_eq!(infer(&empty_remove, &env, &mut errors), Type::Unknown);
}
