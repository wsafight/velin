#[test]
fn wasm_bindings_return_json() {
    let checked = super::check("set x = 1\n");
    assert!(checked.contains("\"ok\":true"), "{checked}");
    let ran = super::run("perform say(\"hi\")\n", "[]");
    assert!(ran.contains("hi"), "{ran}");
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
}
