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
}
