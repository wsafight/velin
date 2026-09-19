use velin::{
    ModuleResolver, ResolvedModule, ScriptRunner, ScriptYield, Value, compile, compile_modules,
};

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

#[test]
fn upcoming_0_5_source_fixture_preserves_composable_dsl_behavior() {
    let source = include_str!("../../../fixtures/compatibility/0.5.0/source.velin");
    let module = include_str!("../../../fixtures/compatibility/0.5.0/math.velin");
    let resolver = |_: &str, specifier: &str| -> Result<ResolvedModule, String> {
        if specifier == "math" {
            Ok(ResolvedModule::new("math", module))
        } else {
            Err(format!("unknown module `{specifier}`"))
        }
    };
    let script = compile_modules("source.velin", source, &resolver as &dyn ModuleResolver).unwrap();
    let mut runner = ScriptRunner::new(&script).unwrap();

    assert_eq!(
        runner.run().unwrap(),
        ScriptYield::Host {
            name: "emit".into(),
            values: vec![Value::Integer(7)],
        }
    );
    assert_eq!(runner.resume(None).unwrap(), ScriptYield::Finished);
}
