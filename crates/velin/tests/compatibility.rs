use velin::{ScriptRunner, ScriptYield, Value, compile};

#[test]
fn released_0_4_source_fixture_preserves_behavior() {
    let source = include_str!("../../../fixtures/compatibility/0.4.0/source.velin");
    let script = compile("source.velin", source).unwrap();
    let mut runner = ScriptRunner::new(&script).unwrap();

    assert_eq!(
        runner.run().unwrap(),
        ScriptYield::Host {
            name: "emit".into(),
            values: vec![Value::Integer(3)],
        }
    );
    assert_eq!(
        runner.resume(None).unwrap(),
        ScriptYield::Host {
            name: "emit".into(),
            values: vec![Value::Integer(5)],
        }
    );
    assert_eq!(runner.resume(None).unwrap(), ScriptYield::Finished);
}
