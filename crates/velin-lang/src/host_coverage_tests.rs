use super::*;
use crate::compile;
use velin_check::{HostSignature, Type};

#[test]
fn schema_insert_and_unknown_command_diagnostics() {
    let mut schema = HostSchema::new();
    assert!(
        schema
            .insert("emit", HostSignature::exact(vec![Type::Integer], None))
            .is_none()
    );
    assert!(schema.get("emit").is_some());
    assert!(!schema.allows_unknown());

    let script = compile("test.velin", "perform emit(1)\nperform other()\n").unwrap();
    let diagnostics = script.check_with_host_schema("test.velin", &schema);
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("`other` is not declared"))
    );

    let schema = HostSchema::new()
        .allow_unknown(true)
        .command("emit", HostSignature::variadic(Vec::new(), Type::Integer, None));
    assert!(schema.allows_unknown());
    let diagnostics = script.check_with_host_schema("test.velin", &schema);
    assert!(diagnostics.iter().all(|diagnostic| !diagnostic.is_error()));
}

#[test]
fn schema_reports_variadic_arity_and_checks_session_values() {
    let script = compile("test.velin", "perform emit(1)\n").unwrap();
    let schema = HostSchema::new().command(
        "emit",
        HostSignature::variadic(vec![Type::Integer, Type::Integer], Type::Integer, None),
    );
    let diagnostics = script.check_with_host_schema("test.velin", &schema);
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("at least"))
    );

    let script = compile("test.velin", "set total = hp + 1\n").unwrap();
    let mut values = BTreeMap::new();
    values.insert("hp".into(), Value::Integer(3));
    let diagnostics = script.check_with_values("test.velin", &values);
    assert!(diagnostics.iter().all(|diagnostic| !diagnostic.is_error()));

    let script = compile("test.velin", "set total = 1\n").unwrap();
    let schema = HostSchema::new().allow_unknown(true);
    let diagnostics = script.check_with_host_schema("test.velin", &schema);
    assert!(diagnostics.iter().all(|diagnostic| !diagnostic.is_error()));

    let mut script = compile("test.velin", "perform emit(1)\n").unwrap();
    script.program = std::sync::Arc::new((*script.program).clone());
    assert!(
        !script
            .validated_program
            .refers_to(&script.program),
        "cloned program must drop the cached validation proof"
    );
    let schema = HostSchema::new().command("emit", HostSignature::exact(vec![Type::Integer], None));
    let _ = script.check_with_host_schema("test.velin", &schema);
}
