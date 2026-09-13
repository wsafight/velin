use std::process::Command;

#[test]
fn runtime_only_example_prints_the_embedded_answer() {
    let output = Command::new(env!("CARGO_BIN_EXE_runtime_only"))
        .output()
        .expect("runtime_only binary");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "42");
}
