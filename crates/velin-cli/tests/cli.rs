//! Integration tests for the `velin` CLI: the reference host runs the shipped
//! `examples/*.velin` files end to end, and the error paths (bad syntax,
//! undefined label, read-before-assignment) surface structured diagnostics.
//!
//! Library entry points (`compile` + `check_script`) cover the same diagnostics
//! the binary prints; the subprocess cases below drive `velin check` / `velin run`
//! so the CLI's argument parser and exit codes are covered too.

use std::io::Write;
use std::process::{Command, Stdio};
use velin::{check_script, compile};

/// Compiles one of the repo's example scripts by path, relative to this crate.
fn example(name: &str) -> String {
    let path = concat_example(name);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {path}: {e}"))
}

fn concat_example(name: &str) -> String {
    format!("{}/../../examples/{name}", env!("CARGO_MANIFEST_DIR"))
}

#[test]
fn shipped_examples_compile_and_pass_checks() {
    for name in ["adventure.velin", "counting.velin"] {
        let source = example(name);
        let script =
            compile(name, &source).unwrap_or_else(|d| panic!("{name} should compile, got: {d}"));
        let diagnostics = check_script(name, &script);
        assert!(
            diagnostics.iter().all(|d| !d.is_error()),
            "{name} should be clean, got: {diagnostics:?}"
        );
    }
}

#[test]
fn a_parse_error_reports_file_line_and_column() {
    // `if` with no `:` is a syntax error; the diagnostic must carry the real
    // file name (not the `<velin>` placeholder) so editors can jump to it.
    let error = compile("broken.velin", "if hp > 0\n    set x = 1\n").unwrap_err();
    assert_eq!(error.file, "broken.velin");
    assert!(error.is_error());
    assert_eq!(error.line, 1);
}

#[test]
fn an_undefined_label_is_a_lowering_error_on_the_jump_line() {
    let error = compile("jump.velin", "perform say(\"hi\")\njump nowhere\n").unwrap_err();
    assert_eq!(error.file, "jump.velin");
    assert!(error.message.contains("nowhere"));
    assert_eq!(error.line, 2);
}

#[test]
fn a_read_before_assignment_is_a_check_error_not_a_compile_error() {
    // This compiles fine — it is only the static checker that flags it, which
    // is exactly why `velin run` checks before executing.
    let script = compile("check.velin", "set total = mystery + 1\n").expect("compiles");
    let diagnostics = check_script("check.velin", &script);
    let error = diagnostics
        .iter()
        .find(|d| d.is_error() && d.message.contains("mystery"))
        .expect("mystery should be flagged");
    assert_eq!(error.file, "check.velin");
}

fn velin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_velin"))
}

fn temp_script(source: &str) -> String {
    let path = std::env::temp_dir().join(format!(
        "velin-cli-int-{}-{}.velin",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::write(&path, source).unwrap();
    path.to_str().unwrap().to_owned()
}

#[test]
fn binary_help_and_unknown_subcommand_exit_2() {
    for args in [
        &["--help"][..],
        &["help"],
        &["-h"],
        &[],
        &["build", "x.velin"],
    ] {
        let output = velin().args(args).output().unwrap();
        assert_eq!(output.status.code(), Some(2), "args={args:?}");
    }
}

#[test]
fn binary_check_and_run_shipped_example() {
    let path = concat_example("counting.velin");
    let check = velin().args(["check", &path]).output().unwrap();
    assert!(
        check.status.success(),
        "{}",
        String::from_utf8_lossy(&check.stderr)
    );
    assert!(String::from_utf8_lossy(&check.stdout).contains("ok"));

    let run = velin().args(["run", &path]).output().unwrap();
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
}

#[test]
fn binary_check_and_run_report_errors() {
    let missing = velin()
        .args(["check", "/no/such/velin.velin"])
        .output()
        .unwrap();
    assert_eq!(missing.status.code(), Some(2));

    let parse_error = temp_script("if hp > 0\n");
    let parse = velin().args(["check", &parse_error]).output().unwrap();
    assert_eq!(parse.status.code(), Some(1));

    let unassigned = temp_script("set total = mystery + 1\n");
    let check = velin().args(["check", &unassigned]).output().unwrap();
    assert_eq!(check.status.code(), Some(1));
    let run = velin().args(["run", &unassigned]).output().unwrap();
    assert_eq!(run.status.code(), Some(1));

    let extra = velin()
        .args(["check", &parse_error, "bonus.velin"])
        .output()
        .unwrap();
    assert_eq!(extra.status.code(), Some(2));
}

#[test]
fn binary_run_executes_ask_and_runtime_errors() {
    let runtime = temp_script("set x = 1 / 0\n");
    let failed = velin().args(["run", &runtime]).output().unwrap();
    assert_eq!(failed.status.code(), Some(1));

    let asking =
        temp_script("choice = perform ask(\"pick:\")\nif choice == 1:\n    perform say(\"one\")\n");
    let mut child = velin()
        .args(["run", &asking])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.as_mut().unwrap().write_all(b"1\n").unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("one"));
}
