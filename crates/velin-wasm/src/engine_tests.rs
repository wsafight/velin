use super::*;

const BRANCHING: &str = "\
default hp = 30
choice = perform ask(\"drink?\")
if choice == 1:
    set hp = hp + 10
    perform say(\"healed\")
else:
    perform say(\"declined\")
";

#[test]
fn check_reports_ok_for_a_clean_script() {
    let result = check("t.velin", "set x = 1\nperform say(x)\n");
    assert!(result.ok);
    assert!(result.diagnostics.is_empty());
}

#[test]
fn check_flattens_a_parse_error() {
    let result = check("t.velin", "if hp > 0\n");
    assert!(!result.ok);
    assert_eq!(result.diagnostics.len(), 1);
    assert_eq!(result.diagnostics[0].severity, "error");
}

#[test]
fn run_answers_ask_from_the_reply_script() {
    let healed = run("t.velin", BRANCHING, vec![Value::Integer(1)]);
    assert!(healed.ok, "{healed:?}");
    assert_eq!(healed.output, vec!["drink?", "healed"]);

    let declined = run("t.velin", BRANCHING, vec![Value::Integer(0)]);
    assert_eq!(declined.output, vec!["drink?", "declined"]);
}

#[test]
fn run_reports_a_check_error_without_executing() {
    let result = run("t.velin", "set total = mystery + 1\n", Vec::new());
    assert!(!result.ok);
    assert!(result.output.is_empty());
    assert!(
        result
            .diagnostics
            .iter()
            .any(|d| d.message.contains("mystery"))
    );
}

#[test]
fn parse_replies_accepts_scalars_and_rejects_the_entire_invalid_input() {
    assert_eq!(
        parse_replies("[1, true, \"hi\"]").unwrap(),
        vec![
            Value::Integer(1),
            Value::Boolean(true),
            Value::String("hi".into())
        ]
    );
    assert!(matches!(
        parse_replies("not json"),
        Err(ParseRepliesError::InvalidJson(_))
    ));
    assert_eq!(
        parse_replies("[1, null, 2]"),
        Err(ParseRepliesError::UnsupportedValue {
            index: 2,
            path: "$".to_owned(),
            message: "null is not a Velin value".to_owned(),
        })
    );
    assert_eq!(
        parse_replies(&" ".repeat(MAX_REPLIES_JSON_BYTES + 1)),
        Err(ParseRepliesError::TooLarge)
    );
    assert!(ParseRepliesError::TooLarge.to_string().contains("1 MiB"));
    assert!(
        parse_replies("{")
            .unwrap_err()
            .to_string()
            .contains("invalid replies JSON")
    );
    assert!(push_line_text(&mut String::new(), "hello", 1).is_err());
}

#[test]
fn host_effect_loops_are_bounded() {
    let result = run(
        "t.velin",
        "while true:\n    perform say(\"loop\")\n",
        Vec::new(),
    );
    assert!(!result.ok);
    assert_eq!(result.output.len(), 1_000);
    assert!(result.error.unwrap().contains("too many host effects"));
}

#[test]
fn run_covers_compile_runtime_and_host_paths() {
    let parse_error = run("t.velin", "if hp > 0\n", Vec::new());
    assert!(!parse_error.ok);
    assert_eq!(parse_error.diagnostics.len(), 1);

    let runtime = run("t.velin", "set x = 1 / 0\n", Vec::new());
    assert!(!runtime.ok);
    assert!(
        runtime
            .error
            .as_deref()
            .is_some_and(|message| message.contains("division by zero"))
    );

    let unknown = run("t.velin", "perform wave(\"flag\")\n", Vec::new());
    assert!(unknown.ok);
    assert_eq!(unknown.output, vec!["[wave] flag"]);

    let compounds = run(
        "t.velin",
        "perform say(list(1, false), record(\"a\", true))\n",
        Vec::new(),
    );
    assert_eq!(compounds.output, vec!["[1, false] {a: true}"]);

    let warning = WireDiagnostic::from(&velin::Diagnostic::warning("t.velin", 1, 1, "unused"));
    assert_eq!(warning.severity, "warning");

    assert!(matches!(
        parse_replies("[1.5, null, {}, []]"),
        Err(ParseRepliesError::UnsupportedValue { index: 1, .. })
    ));
    assert!(parse_replies("[]").unwrap().is_empty());
}

#[test]
fn parse_replies_preserves_lists_records_order_and_nested_error_paths() {
    let replies = parse_replies(r#"[[1, true], {"z": 2, "a": [3]}]"#).unwrap();
    assert_eq!(
        replies[0],
        Value::List(std::sync::Arc::new(vec![
            Value::Integer(1),
            Value::Boolean(true),
        ]))
    );
    let Value::Record(record) = &replies[1] else {
        panic!("expected record")
    };
    assert_eq!(
        record.keys().map(String::as_str).collect::<Vec<_>>(),
        vec!["a", "z"]
    );

    let error = parse_replies(r#"[{"items": [1, null]}]"#).unwrap_err();
    assert!(matches!(
        error,
        ParseRepliesError::UnsupportedValue {
            index: 1,
            ref path,
            ..
        } if path == "$[\"items\"][1]"
    ));
}

#[test]
fn output_budget_is_enforced() {
    let payload = "x".repeat(2_100);
    let source = format!("set msg = \"{payload}\"\nwhile true:\n    perform say(msg)\n");
    let result = run("t.velin", &source, Vec::new());
    assert!(!result.ok);
    assert!(
        result
            .error
            .as_deref()
            .is_some_and(|message| message.contains("output exceeds 1 MiB"))
    );
}
