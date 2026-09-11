//! End-to-end test of the **surface language**: a real `.velin` script string
//! is compiled with [`velin::compile`], statically checked, then run through a
//! toy host — the full parse → lower → check → run path an embedder uses.

use std::collections::BTreeMap;
use velin::{
    HostSchema, HostSignature, Machine, Type, Value, Yield, check_script,
    check_script_with_host_schema, compile,
};

/// A tiny host that records every effect and answers `ask` effects from a fixed
/// script of replies keyed by call order.
#[derive(Default)]
struct ToyHost {
    /// `command name → replay of resume values` for effects that return data.
    replies: BTreeMap<String, Vec<Value>>,
    log: Vec<String>,
}

impl ToyHost {
    /// Runs `script` to completion, dispatching host effects by name.
    fn run(&mut self, script: &velin::CompiledScript) -> Machine {
        let mut machine = Machine::new(script.program.clone()).unwrap();
        for (name, value) in &script.defaults {
            machine.set_variable(name, value.clone());
        }
        let mut outcome = machine.run().unwrap();
        loop {
            match outcome {
                Yield::Finished => break,
                Yield::Host { host_id, values } => {
                    let name = script.host_name(host_id).unwrap().to_owned();
                    self.log.push(format!("{name}{values:?}"));
                    let reply = self.replies.get_mut(&name).and_then(Vec::pop);
                    outcome = machine.resume(reply).unwrap();
                }
            }
        }
        machine
    }
}

const SCRIPT: &str = "\
# A tiny branching adventure written in Velin's surface language.
default hp = 30
default potions = 2

label start:
    perform say(\"You wake in a cold cell.\")
    choice = perform ask(\"Drink a potion?\")
    if choice == 1:
        set hp = hp + 10
        set potions = potions - 1
        perform say(\"You feel better.\")
    else:
        perform say(\"You steel yourself.\")

    while potions > 0:
        set potions = potions - 1
        perform say(\"You stash a potion.\")

    if hp > 35:
        jump good_end
    perform say(\"You limp onward.\")

label good_end:
    perform say(\"You escape, unbroken.\")
";

#[test]
fn compiles_checks_and_runs_a_surface_language_script() {
    let script = compile("adventure.velin", SCRIPT).expect("script compiles");

    // The host table is populated by first-seen order of `perform` commands.
    assert!(script.hosts.contains(&"say".to_string()));
    assert!(script.hosts.contains(&"ask".to_string()));
    assert_eq!(script.defaults.get("hp"), Some(&Value::Integer(30)));

    // Static checks: definite assignment (defaults seed hp/potions) and type
    // inference over embedded expressions. The script is written correctly, so
    // there should be no errors.
    let diagnostics = check_script("adventure.velin", &script);
    assert!(
        diagnostics.iter().all(|d| !d.is_error()),
        "unexpected errors: {diagnostics:?}"
    );

    // Drive it: answer the one `ask` with choice 1 (drink the potion).
    let mut host = ToyHost::default();
    host.replies.insert("ask".into(), vec![Value::Integer(1)]);
    let machine = host.run(&script);

    // choice == 1 → hp 30 + 10 = 40; potions started at 2, one drunk (→1),
    // then the while loop stashes the last one (→0).
    assert_eq!(machine.variable("hp"), Some(&Value::Integer(40)));
    assert_eq!(machine.variable("potions"), Some(&Value::Integer(0)));

    // hp (40) > 35, so we jump to good_end and skip the "limp onward" line.
    assert!(host.log.iter().any(|l| l.contains("feel better")));
    assert!(host.log.iter().any(|l| l.contains("stash a potion")));
    assert!(host.log.iter().all(|l| !l.contains("limp onward")));
    assert!(host.log.last().unwrap().contains("escape, unbroken"));
}

#[test]
fn other_branch_runs_when_the_host_declines() {
    let script = compile("adventure.velin", SCRIPT).expect("script compiles");

    // Answer the `ask` with 0 (decline): the `else` branch runs, hp stays 30.
    let mut host = ToyHost::default();
    host.replies.insert("ask".into(), vec![Value::Integer(0)]);
    let machine = host.run(&script);

    assert_eq!(machine.variable("hp"), Some(&Value::Integer(30)));
    // Both potions are stashed by the loop (none drunk).
    assert_eq!(machine.variable("potions"), Some(&Value::Integer(0)));
    assert!(host.log.iter().any(|l| l.contains("steel yourself")));
    // hp (30) is not > 35, so the "limp onward" line runs before good_end.
    assert!(host.log.iter().any(|l| l.contains("limp onward")));
}

#[test]
fn a_read_before_assignment_is_reported_by_check_script() {
    // `mystery` is read but never assigned or defaulted: definite-assignment
    // analysis should flag it without the program having to run.
    let script = compile("check.velin", "set total = mystery + 1\n").expect("compiles");
    let diagnostics = check_script("check.velin", &script);
    assert!(
        diagnostics.iter().any(|d| d.message.contains("mystery")),
        "expected a diagnostic mentioning `mystery`: {diagnostics:?}"
    );
    assert_eq!(
        diagnostics
            .iter()
            .find(|diagnostic| diagnostic.message.contains("mystery"))
            .unwrap()
            .column,
        13
    );
}

#[test]
fn known_non_boolean_conditions_are_rejected_by_check_script() {
    let script = compile("check.velin", "if 1:\n    set x = 2\n").expect("compiles");
    let diagnostics = check_script("check.velin", &script);
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("condition expects boolean"))
    );
}

#[test]
fn type_checking_propagates_defaults_and_reports_the_operator_column() {
    let script = compile("types.velin", "default x = 1\nset y = x + \"a\"\n").unwrap();
    let diagnostic = check_script("types.velin", &script)
        .into_iter()
        .find(|diagnostic| diagnostic.message.contains("cannot combine"))
        .expect("known default type makes the addition invalid");
    assert_eq!((diagnostic.line, diagnostic.column), (2, 11));
}

#[test]
fn type_checking_merges_branches_and_skips_unreachable_sites() {
    let same_type = compile(
        "types.velin",
        "default flag = true\nif flag:\n    set x = 1\nelse:\n    set x = 2\nset y = x + \"a\"\n",
    )
    .unwrap();
    assert!(
        check_script("types.velin", &same_type)
            .iter()
            .any(|diagnostic| diagnostic.message.contains("cannot combine"))
    );

    let unreachable = compile(
        "types.velin",
        "jump done\nif 1:\n    set x = 2\nlabel done:\n",
    )
    .unwrap();
    assert!(check_script("types.velin", &unreachable).is_empty());
}

#[test]
fn type_checking_handles_loops_conflicts_and_host_bindings() {
    let stable_loop = compile(
        "types.velin",
        "default x = 1\ndefault flag = true\nwhile flag:\n    set x = 2\n    set flag = false\nset y = x + \"a\"\n",
    )
    .unwrap();
    assert!(
        check_script("types.velin", &stable_loop)
            .iter()
            .any(|diagnostic| diagnostic.message.contains("cannot combine"))
    );

    let conflicting_branches = compile(
        "types.velin",
        "default flag = true\nif flag:\n    set x = 1\nelse:\n    set x = \"a\"\nset y = x + 1\n",
    )
    .unwrap();
    assert!(check_script("types.velin", &conflicting_branches).is_empty());

    let host_binding = compile(
        "types.velin",
        "answer = perform ask(\"number?\")\nset y = answer + 1\n",
    )
    .unwrap();
    assert!(check_script("types.velin", &host_binding).is_empty());
}

#[test]
fn surface_random_calls_are_seeded_and_pass_static_checks() {
    let script = compile(
        "random.velin",
        "set die = random(1, 6)\nset hit = chance(100)\nset miss = chance(0)\n",
    )
    .expect("script compiles");
    assert!(check_script("random.velin", &script).is_empty());

    let mut first = Machine::with_seed(script.program.clone(), 123).unwrap();
    let mut replay = Machine::with_seed(script.program.clone(), 123).unwrap();
    assert_eq!(first.run().unwrap(), Yield::Finished);
    assert_eq!(replay.run().unwrap(), Yield::Finished);
    assert_eq!(first.variable("die"), replay.variable("die"));
    assert_eq!(first.variable("hit"), Some(&Value::Boolean(true)));
    assert_eq!(first.variable("miss"), Some(&Value::Boolean(false)));
    assert_eq!(first.rng_state(), replay.rng_state());
}

#[test]
fn host_schema_checks_names_arity_arguments_bindings_and_return_flow() {
    let schema = HostSchema::new()
        .command(
            "ask_number",
            HostSignature::exact(vec![Type::String], Some(Type::Integer)),
        )
        .command(
            "say",
            HostSignature::variadic(Vec::new(), Type::Unknown, None),
        );
    let script = compile(
        "host.velin",
        "answer = perform ask_number(1, 2)\n\
         set bad = answer + \"x\"\n\
         captured = perform say(\"hello\")\n\
         perform missing()\n",
    )
    .unwrap();
    assert!(check_script("host.velin", &script).is_empty());

    let diagnostics = check_script_with_host_schema("host.velin", &script, &schema);
    let messages: Vec<&str> = diagnostics
        .iter()
        .map(|diagnostic| diagnostic.message.as_str())
        .collect();
    assert!(
        messages
            .iter()
            .any(|message| message.contains("expects 1 argument"))
    );
    assert!(
        messages
            .iter()
            .any(|message| message.contains("host argument 1 expects string"))
    );
    assert!(
        messages
            .iter()
            .any(|message| message.contains("cannot combine"))
    );
    assert!(
        messages
            .iter()
            .any(|message| message.contains("does not return"))
    );
    assert!(
        messages
            .iter()
            .any(|message| message.contains("not declared"))
    );

    let permissive = schema.clone().allow_unknown(true);
    let diagnostics = check_script_with_host_schema("host.velin", &script, &permissive);
    assert!(
        diagnostics
            .iter()
            .all(|diagnostic| !diagnostic.message.contains("not declared"))
    );
}
