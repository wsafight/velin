use std::collections::BTreeMap;
use velin::{Machine, ModuleResolver, ResolvedModule, Value, Yield, compile_modules};

struct Resolver(BTreeMap<String, String>);

impl ModuleResolver for Resolver {
    fn resolve(&self, _importer: &str, specifier: &str) -> Result<ResolvedModule, String> {
        self.0
            .get(specifier)
            .map(|source| ResolvedModule::new(specifier, source.clone()))
            .ok_or_else(|| format!("missing module `{specifier}`"))
    }
}

#[test]
fn multi_file_pure_functions_execute_in_the_main_state_machine() {
    let resolver = Resolver(BTreeMap::from([
        (
            "math".into(),
            "fn twice(value):\n    return value * 2\n\
             export fn score(values):\n    set total = 0\n    for value in values:\n        total += value\n    set doubled = call twice(total)\n    return doubled\n"
                .into(),
        ),
    ]));
    let script = compile_modules(
        "main",
        "import math\nset result = call math.score([1, 2, 3])\n",
        &resolver,
    )
    .unwrap();
    assert!(script.check("main").is_empty());
    let mut machine = Machine::new(script.program.clone()).unwrap();
    assert!(matches!(machine.run().unwrap(), Yield::Finished));
    assert_eq!(machine.variable("result"), Some(&Value::Integer(12)));
}
