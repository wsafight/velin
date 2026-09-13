use super::*;

#[test]
fn formats_errors_and_warnings_with_optional_hints() {
    let error = Diagnostic::new("script.rl", 4, 8, "unknown symbol");
    assert!(error.is_error());
    assert_eq!(error.to_string(), "script.rl:4:8: error: unknown symbol");

    let warning = Diagnostic::warning("script.rl", 2, 1, "unused").with_hint("delete the binding");
    assert!(!warning.is_error());
    assert_eq!(
        warning.to_string(),
        "script.rl:2:1: warning: unused\n  hint: delete the binding"
    );
}
