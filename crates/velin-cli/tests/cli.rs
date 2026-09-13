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
fn binary_help_and_version_succeed_while_invalid_commands_exit_2() {
    for args in [&["--help"][..], &["help"], &["-h"]] {
        let output = velin().args(args).output().unwrap();
        assert!(output.status.success(), "args={args:?}");
        assert!(String::from_utf8_lossy(&output.stdout).contains("usage:"));
    }
    for args in [&["--version"][..], &["-V"]] {
        let output = velin().args(args).output().unwrap();
        assert!(output.status.success(), "args={args:?}");
        assert_eq!(
            String::from_utf8_lossy(&output.stdout).trim(),
            concat!("velin ", env!("CARGO_PKG_VERSION"))
        );
    }
    for args in [&[][..], &["build", "x.velin"]] {
        let output = velin().args(args).output().unwrap();
        assert_eq!(output.status.code(), Some(2), "args={args:?}");
    }
}

#[test]
fn binary_reads_source_from_stdin() {
    let mut check = velin()
        .args(["check", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    check
        .stdin
        .take()
        .unwrap()
        .write_all(b"set value = 1\n")
        .unwrap();
    let checked = check.wait_with_output().unwrap();
    assert!(checked.status.success());
    assert!(String::from_utf8_lossy(&checked.stdout).contains("<stdin>: ok"));

    let mut run = velin()
        .args(["run", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    run.stdin
        .take()
        .unwrap()
        .write_all(b"perform say(\"from stdin\")\n")
        .unwrap();
    let output = run.wait_with_output().unwrap();
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("from stdin"));
}

#[test]
fn binary_check_json_is_machine_readable_for_success_and_failure() {
    let clean = temp_script("set value = 1\n");
    let checked = velin().args(["check", "--json", &clean]).output().unwrap();
    assert!(checked.status.success());
    let payload: serde_json::Value = serde_json::from_slice(&checked.stdout).unwrap();
    assert_eq!(payload["ok"], true);
    assert_eq!(payload["diagnostics"], serde_json::json!([]));

    let broken = temp_script("set total = missing + 1\n");
    let checked = velin().args(["check", &broken, "--json"]).output().unwrap();
    assert_eq!(checked.status.code(), Some(1));
    let payload: serde_json::Value = serde_json::from_slice(&checked.stdout).unwrap();
    assert_eq!(payload["ok"], false);
    assert_eq!(payload["diagnostics"][0]["severity"], "error");
    assert!(
        payload["diagnostics"][0]["message"]
            .as_str()
            .unwrap()
            .contains("missing")
    );
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

fn temp_path(suffix: &str) -> String {
    let path = std::env::temp_dir().join(format!(
        "velin-cli-int-{}-{suffix}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    path.to_str().unwrap().to_owned()
}

#[test]
fn binary_compile_writes_an_artifact_that_check_and_run_accept() {
    let source = temp_script("perform say(\"artifact\")\n");
    let output = temp_path("out.velinc");
    let compiled = velin()
        .args(["compile", &source, "-o", &output])
        .output()
        .unwrap();
    assert!(
        compiled.status.success(),
        "{}",
        String::from_utf8_lossy(&compiled.stderr)
    );

    let checked = velin().args(["check", &output]).output().unwrap();
    assert!(checked.status.success());
    assert!(String::from_utf8_lossy(&checked.stdout).contains("ok"));

    let json = velin()
        .args(["check", "--json", &output])
        .output()
        .unwrap();
    assert!(json.status.success());
    let payload: serde_json::Value = serde_json::from_slice(&json.stdout).unwrap();
    assert_eq!(payload["ok"], true);

    let run = velin().args(["run", &output]).output().unwrap();
    assert!(run.status.success());
    assert!(String::from_utf8_lossy(&run.stdout).contains("artifact"));
    let _ = std::fs::remove_file(output);
}

#[test]
fn binary_compile_rejects_bad_arguments_and_inputs() {
    let source = temp_script("set value = 1\n");
    let output = temp_path("out.velinc");

    let stdin = velin()
        .args(["compile", "-", "-o", &output])
        .output()
        .unwrap();
    assert_eq!(stdin.status.code(), Some(2));

    let missing = velin()
        .args(["compile", "/no/such/velin.velin", "-o", &output])
        .output()
        .unwrap();
    assert_eq!(missing.status.code(), Some(2));

    let parse_error = temp_script("if hp > 0\n");
    let parsed = velin()
        .args(["compile", &parse_error, "-o", &output])
        .output()
        .unwrap();
    assert_eq!(parsed.status.code(), Some(1));

    let unassigned = temp_script("set total = mystery + 1\n");
    let checked = velin()
        .args(["compile", &unassigned, "-o", &output])
        .output()
        .unwrap();
    assert_eq!(checked.status.code(), Some(1));

    for args in [
        &["compile"][..],
        &["compile", "--oops", "-o", "out.velinc"],
        &["compile", &source, "not-o", &output],
        &["compile", &source, "-o"],
        &["compile", &source, "-o", "--oops"],
        &["compile", &source, "-o", &output, "extra"],
        &["check", "--unknown", &source],
        &["check", "--json", "--json", &source],
        &["run", "--unknown", &source],
        &["run", &source, "extra"],
        &["--help", "extra"],
        &["--version", "extra"],
    ] {
        let failed = velin().args(args.iter().copied()).output().unwrap();
        assert_eq!(failed.status.code(), Some(2), "args={args:?}");
    }
}

#[test]
fn binary_check_json_reports_io_and_artifact_failures() {
    let missing = velin()
        .args(["check", "--json", "/no/such/velin.velin"])
        .output()
        .unwrap();
    assert_eq!(missing.status.code(), Some(2));
    let payload: serde_json::Value = serde_json::from_slice(&missing.stdout).unwrap();
    assert_eq!(payload["ok"], false);

    let invalid_utf8 = temp_path("invalid.velin");
    std::fs::write(&invalid_utf8, [0xff, 0xfe]).unwrap();
    let utf8 = velin().args(["check", &invalid_utf8]).output().unwrap();
    assert_eq!(utf8.status.code(), Some(2));
    let utf8_run = velin().args(["run", &invalid_utf8]).output().unwrap();
    assert_eq!(utf8_run.status.code(), Some(2));
    let utf8_json = velin()
        .args(["check", "--json", &invalid_utf8])
        .output()
        .unwrap();
    assert_eq!(utf8_json.status.code(), Some(2));

    let broken_artifact = temp_path("broken.velinc");
    let mut bytes = b"VELINBC\0".to_vec();
    bytes.extend_from_slice(&3_u16.to_le_bytes());
    bytes.extend_from_slice(&0_u64.to_le_bytes());
    std::fs::write(&broken_artifact, bytes).unwrap();
    let artifact = velin().args(["check", &broken_artifact]).output().unwrap();
    assert_eq!(artifact.status.code(), Some(1));
    let artifact_json = velin()
        .args(["check", "--json", &broken_artifact])
        .output()
        .unwrap();
    assert_eq!(artifact_json.status.code(), Some(1));
    let run = velin().args(["run", &broken_artifact]).output().unwrap();
    assert_eq!(run.status.code(), Some(1));
    let _ = std::fs::remove_file(invalid_utf8);
    let _ = std::fs::remove_file(broken_artifact);
}

#[test]
fn binary_check_hits_the_artifact_cache_and_compile_write_errors() {
    let source = temp_script("set value = 1\n");
    let first = velin().args(["check", &source]).output().unwrap();
    assert!(first.status.success());
    let second = velin().args(["check", &source]).output().unwrap();
    assert!(second.status.success());

    let parent = temp_path("parent");
    std::fs::write(&parent, b"not a directory").unwrap();
    let output = format!("{parent}/out.velinc");
    let failed = velin()
        .args(["compile", &source, "-o", &output])
        .output()
        .unwrap();
    assert_eq!(failed.status.code(), Some(2));
    let _ = std::fs::remove_file(parent);
}
