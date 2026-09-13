    use super::*;
    use std::io::Cursor;
    use velin::compile;

    fn run_capture(source: &str, stdin: &str) -> String {
        let script = compile("test.velin", source).expect("compiles");
        let mut input = Cursor::new(stdin.to_owned());
        let mut output = Vec::new();
        run(&script, &mut input, &mut output).expect("runs");
        String::from_utf8(output).expect("utf8")
    }

    #[test]
    fn host_effect_loops_are_bounded() {
        let script =
            compile("test.velin", "while true:\n    perform say(\"loop\")\n").expect("compiles");
        let mut input = Cursor::new(String::new());
        let mut output = Vec::new();
        let error = run(&script, &mut input, &mut output).unwrap_err();
        assert!(matches!(
            error,
            RunError::Script(ScriptRunError::HostEffectsExceeded { limit: 1_000 })
        ));
        assert_eq!(output.lines().count(), 1_000);
    }

    #[test]
    fn say_writes_arguments_joined() {
        let out = run_capture("perform say(\"hello\", 42)\n", "");
        assert_eq!(out, "hello 42\n");
    }

    #[test]
    fn ask_reads_a_reply_and_binds_it() {
        let source = "\
choice = perform ask(\"pick:\")
if choice == 2:
    perform say(\"two\")
else:
    perform say(\"other\")
";
        assert_eq!(run_capture(source, "2\n"), "pick: two\n");
        assert_eq!(run_capture(source, "9\n"), "pick: other\n");
    }

    #[test]
    fn unmodelled_command_is_echoed_as_an_effect() {
        let out = run_capture("perform wave(\"flag\")\n", "");
        assert_eq!(out, "[wave] flag\n");
    }

    #[test]
    fn reply_parses_integers_booleans_and_strings() {
        assert_eq!(parse_reply("7"), Value::Integer(7));
        assert_eq!(parse_reply("true"), Value::Boolean(true));
        assert_eq!(parse_reply("false"), Value::Boolean(false));
        assert_eq!(parse_reply("hi"), Value::String("hi".into()));
    }

    #[test]
    fn run_error_display_and_io_conversion() {
        let eval = RunError::Script(ScriptRunError::Evaluation(velin::EvalError::new(4, "boom")));
        assert_eq!(eval.to_string(), "runtime error at line 4: boom");
        let io = RunError::from(std::io::Error::other("disk"));
        assert!(io.to_string().contains("i/o error"));
        assert_eq!(
            RunError::Budget("too many host effects").to_string(),
            "execution budget exceeded: too many host effects"
        );
    }

    #[test]
    fn ask_eof_list_record_and_runtime_errors() {
        let script = compile("test.velin", "choice = perform ask(\"pick:\")\n").expect("compiles");
        let mut input = Cursor::new(String::new());
        let mut output = Vec::new();
        let error = run(&script, &mut input, &mut output).unwrap_err();
        assert!(matches!(
            error,
            RunError::Script(ScriptRunError::Evaluation(_))
        ));
        assert!(String::from_utf8(output).unwrap().contains("pick:"));

        let compounds = run_capture("perform say(list(1, true), record(\"a\", \"b\"))\n", "");
        assert_eq!(compounds, "[1, true] {a: b}\n");

        let script = compile("test.velin", "set x = 1 / 0\n").expect("compiles");
        let mut input = Cursor::new(String::new());
        let mut output = Vec::new();
        let error = run(&script, &mut input, &mut output).unwrap_err();
        assert!(matches!(
            error,
            RunError::Script(ScriptRunError::Evaluation(_))
        ));
        assert!(error.to_string().contains("division by zero"));
    }

    #[test]
    fn output_budget_is_enforced() {
        let payload = "x".repeat(2_100);
        let source = format!("set msg = \"{payload}\"\nwhile true:\n    perform say(msg)\n");
        let script = compile("test.velin", &source).expect("compiles");
        let mut input = Cursor::new(String::new());
        let mut output = Vec::new();
        let error = run(&script, &mut input, &mut output).unwrap_err();
        assert!(matches!(error, RunError::Budget("output exceeds 1 MiB")));
        let mut written = 0;
        assert!(write_bounded(
            &mut Vec::new(),
            &"x".repeat(MAX_OUTPUT_BYTES),
            "\n",
            &mut written
        )
        .is_err());
        assert!(push_text_bounded(&mut String::new(), "hello", 1).is_err());
    }
