use super::*;

fn temp_script(source: &str) -> String {
    let path = std::env::temp_dir().join(format!(
        "velin-cli-{}-{}.velin",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    std::fs::write(&path, source).expect("write temp script");
    path.to_str().expect("utf8 path").to_owned()
}
#[test]
fn parse_args_accepts_check_and_run_and_rejects_the_rest() {
    match parse_args(["check".into(), "--json".into(), "a.velin".into()].into_iter()) {
        Ok(Command::Check { path, json }) => {
            assert_eq!(path, "a.velin");
            assert!(json);
        }
        Ok(
            Command::Run(_)
            | Command::Format { .. }
            | Command::Compile { .. }
            | Command::Help
            | Command::Version,
        ) => {
            panic!("expected check")
        }
        Err(message) => panic!("expected check, got error {message}"),
    }
    match parse_args(["run".into(), "b.velin".into()].into_iter()) {
        Ok(Command::Run(path)) => assert_eq!(path, "b.velin"),
        Ok(
            Command::Check { .. }
            | Command::Format { .. }
            | Command::Compile { .. }
            | Command::Help
            | Command::Version,
        ) => {
            panic!("expected run")
        }
        Err(message) => panic!("expected run, got error {message}"),
    }
    assert!(parse_args(std::iter::empty()).is_err());
    assert!(matches!(
        parse_args(["--help".into()].into_iter()),
        Ok(Command::Help)
    ));
    assert!(matches!(
        parse_args(["--version".into()].into_iter()),
        Ok(Command::Version)
    ));
    assert!(parse_args(["check".into()].into_iter()).is_err());
    assert!(parse_args(["check".into(), "a".into(), "extra".into()].into_iter()).is_err());
    assert!(parse_args(["build".into(), "a.velin".into()].into_iter()).is_err());
    assert!(matches!(
        parse_args(["fmt".into(), "--check".into(), "a.velin".into()].into_iter()),
        Ok(Command::Format { check: true, .. })
    ));
}

#[test]
fn format_command_writes_and_checks_canonical_source() {
    let path = temp_script("set value=1+2\n");
    assert_eq!(
        run(Command::Format {
            path: path.clone(),
            check: true,
        }),
        ExitCode::FAILURE
    );
    assert_eq!(
        run(Command::Format {
            path: path.clone(),
            check: false,
        }),
        ExitCode::SUCCESS
    );
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "set value = 1 + 2\n"
    );
    assert_eq!(
        run(Command::Format { path, check: true }),
        ExitCode::SUCCESS
    );
}
#[test]
fn check_and_execute_cover_success_and_failure_paths() {
    let ok = temp_script("set x = 1\nperform say(x)\n");
    let parse_error = temp_script("if hp > 0\n");
    let check_error = temp_script("set total = mystery + 1\n");
    let runtime_error = temp_script("set x = 1 / 0\n");

    let _ = run(Command::Check {
        path: ok.clone(),
        json: false,
    });
    let _ = run(Command::Run(ok));
    let _ = check(&parse_error, false);
    let _ = check(&parse_error, true);
    let _ = execute(&parse_error);
    let _ = check(&check_error, false);
    let _ = execute(&check_error);
    let _ = execute(&runtime_error);
    let _ = check("/no/such/velin-file.velin", false);
    let _ = execute("/no/such/velin-file.velin");
    let _ = read("/no/such/velin-file.velin");
    assert_eq!(run(Command::Help), ExitCode::SUCCESS);
    assert_eq!(run(Command::Version), ExitCode::SUCCESS);
    let unique = temp_script("set unique_cli_cache = 1\n");
    let source = std::fs::read_to_string(&unique).unwrap();
    compile_source_cached(&unique, "t.velin", &source).unwrap();
    let huge = vec![b'x'; MAX_SOURCE_BYTES + 1];
    assert!(read_source(std::io::Cursor::new(huge), "t").is_err());
    let huge = vec![b'x'; 8];
    assert!(read_bounded(std::io::Cursor::new(huge), "t", 4).is_err());
}
