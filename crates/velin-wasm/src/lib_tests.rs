#[test]
fn wasm_bindings_return_json() {
    let checked = super::check("set x = 1\n");
    assert!(checked.contains("\"ok\":true"), "{checked}");
    let ran = super::run("perform say(\"hi\")\n", "[]");
    assert!(ran.contains("hi"), "{ran}");
    let bounded = super::run_with_policy(
        "while true:\n    set value = 1\n",
        "[]",
        r#"{"max_fuel":8,"max_immediate_fuel":8}"#,
    );
    assert!(bounded.contains("fuel"), "{bounded}");
    let replies = super::run(
        "choice = perform ask(\"go?\")\nperform say(choice)\n",
        "[1]",
    );
    assert!(replies.contains('1'), "{replies}");
    let garbage = super::run("perform say(\"must not run\")\n", "not-json");
    assert!(garbage.contains("invalid replies JSON"), "{garbage}");
    assert!(!garbage.contains("must not run"), "{garbage}");

    let shifted = super::run(
        "first = perform ask(\"first\")\nsecond = perform ask(\"second\")\n",
        "[1, null, 2]",
    );
    assert!(shifted.contains("item 2"), "{shifted}");
    assert!(!shifted.contains("first\""), "{shifted}");

    let mut session = super::PlaygroundSession::new("perform say(\"debug\")\n");
    assert!(session.state().contains("\"status\":\"paused\""));
    assert!(session.snapshot().contains("\"snapshot\":1"));
    assert!(session.resume("[]").contains("debug"));
    assert!(session.restore(1).contains("\"status\":\"paused\""));
    let mut bounded_session = super::PlaygroundSession::new_with_policy(
        "while true:\n    set value = 1\n",
        r#"{"max_fuel":8}"#,
    );
    assert!(bounded_session.cancel().contains("\"ok\":true"));
    assert!(bounded_session.resume("[]").contains("cancelled"));
}

#[cfg(feature = "runtime")]
#[test]
fn runtime_binding_accepts_compounds_and_caps_resume_json() {
    let script = velin::compile(
        "runtime.velin",
        "answer = perform ask()\nperform emit(answer)\n",
    )
    .unwrap();
    let program = serde_json::to_string(&*script.program).unwrap();
    let mut machine = super::RuntimeMachine::new(&program).unwrap();
    assert!(machine.run().contains("\"kind\":\"host\""));
    let resumed = machine.resume(r#"{"items":[1,true]}"#);
    assert!(resumed.contains(r#""values":[{"items":[1,true]}]"#));

    let script = velin::compile("runtime.velin", "answer = perform ask()\n").unwrap();
    let program = serde_json::to_string(&*script.program).unwrap();
    let mut machine = super::RuntimeMachine::new(&program).unwrap();
    let _ = machine.run();
    let oversized = " ".repeat(super::MAX_RUNTIME_VALUE_JSON_BYTES + 1);
    assert!(machine.resume(&oversized).contains("JSON exceeds 1 MiB"));

    let mut bounded = super::RuntimeMachine::new_with_policy(
        &serde_json::to_string(
            &*velin::compile(
                "runtime.velin",
                "set counter = 0\nwhile true:\n    set counter = counter + 1\n",
            )
            .unwrap()
            .program,
        )
        .unwrap(),
        r#"{"max_fuel":8,"max_immediate_fuel":8}"#,
    )
    .unwrap();
    let bounded_result = bounded.run();
    assert!(bounded_result.contains("fuel"), "{bounded_result}");
}

#[cfg(feature = "runtime")]
#[test]
fn runtime_program_reuses_validation_across_machines() {
    let script = velin::compile("runtime.velin", "perform emit(1)\n").unwrap();
    let program_json = serde_json::to_string(&*script.program).unwrap();
    let program = super::RuntimeProgram::new(&program_json).unwrap();

    let mut first = program.create_machine();
    let mut second = program
        .create_machine_with_policy(r#"{"max_host_effects":4}"#)
        .unwrap();

    assert!(first.run_batch(1).contains(r#""host_id":0"#));
    assert!(second.run_batch(1).contains(r#""host_id":0"#));
}
