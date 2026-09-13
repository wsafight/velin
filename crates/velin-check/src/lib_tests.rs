use super::*;
use velin_parse::parse_expression;

#[test]
fn check_expression_emits_structured_diagnostics() {
    let expr = parse_expression("\"a\" + 1", "script.ql", 3, 1).unwrap();
    let diagnostics = check_expression(&expr, &Environment::new(), "script.ql", 3, 1);
    assert_eq!(diagnostics.len(), 1);
    assert!(diagnostics[0].is_error());
    assert_eq!(diagnostics[0].line, 3);
    assert!(diagnostics[0].message.contains("`+`"));
}

#[test]
fn check_condition_rejects_a_known_non_boolean() {
    let expr = parse_expression("1", "script.ql", 3, 4).unwrap();
    let diagnostics = check_condition(&expr, &Environment::new(), "script.ql", 3, 4);
    assert_eq!(diagnostics.len(), 1);
    assert!(diagnostics[0].message.contains("condition expects boolean"));
}
