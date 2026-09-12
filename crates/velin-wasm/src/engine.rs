//! The Playground engine: `.velin` source in, serializable result out.
//!
//! Kept free of any `wasm_bindgen` attributes so it compiles and unit-tests on
//! the host. The wasm layer in `lib.rs` is a one-line string wrapper over each
//! function here. A *scripted* host makes a run deterministic: `say` appends a
//! line to the output log, and `ask` is answered from a fixed reply list, so
//! the same source + replies always produce the same transcript.

use serde::Serialize;
use std::fmt;
use velin::{Diagnostic, ScriptRunner, ScriptYield, Value, check_script, compile};

const MAX_OUTPUT_BYTES: usize = 1024 * 1024;
const MAX_REPLIES_JSON_BYTES: usize = 1024 * 1024;

/// A caller error in the scripted replies supplied to the Playground host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseRepliesError {
    TooLarge,
    InvalidJson(String),
    UnsupportedValue { index: usize },
}

impl fmt::Display for ParseRepliesError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooLarge => write!(formatter, "invalid replies: JSON exceeds 1 MiB"),
            Self::InvalidJson(message) => write!(formatter, "invalid replies JSON: {message}"),
            Self::UnsupportedValue { index } => write!(
                formatter,
                "invalid replies: item {index} must be an integer, boolean, or string"
            ),
        }
    }
}

/// The result of [`check`]: whether the script is error-free plus every
/// diagnostic (errors and warnings), pre-flattened for the browser.
#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct CheckResult {
    pub ok: bool,
    pub diagnostics: Vec<WireDiagnostic>,
}

/// The result of [`run`]: the check outcome, the lines `say` produced, and the
/// runtime error message if execution failed.
#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct RunResult {
    pub ok: bool,
    pub diagnostics: Vec<WireDiagnostic>,
    pub output: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// A diagnostic flattened to the fields the web UI renders.
#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct WireDiagnostic {
    pub severity: String,
    pub line: usize,
    pub column: usize,
    pub message: String,
}

impl WireDiagnostic {
    fn from(diagnostic: &Diagnostic) -> Self {
        Self {
            severity: if diagnostic.is_error() {
                "error".to_owned()
            } else {
                "warning".to_owned()
            },
            line: diagnostic.line,
            column: diagnostic.column,
            message: diagnostic.message.clone(),
        }
    }
}

/// Compiles and statically checks `source`, collecting all diagnostics.
#[must_use]
pub fn check(file: &str, source: &str) -> CheckResult {
    let diagnostics = diagnose(file, source);
    CheckResult {
        ok: diagnostics.iter().all(|d| d.severity != "error"),
        diagnostics,
    }
}

/// Checks `source`, then (if it has no errors) runs it against the scripted
/// host, answering `ask` effects from `replies` in order.
#[must_use]
pub fn run(file: &str, source: &str, replies: Vec<Value>) -> RunResult {
    let script = match compile(file, source) {
        Ok(script) => script,
        Err(diagnostic) => {
            return RunResult {
                ok: false,
                diagnostics: vec![WireDiagnostic::from(&diagnostic)],
                output: Vec::new(),
                error: None,
            };
        }
    };

    let diagnostics: Vec<WireDiagnostic> = check_script(file, &script)
        .iter()
        .map(WireDiagnostic::from)
        .collect();
    if diagnostics.iter().any(|d| d.severity == "error") {
        return RunResult {
            ok: false,
            diagnostics,
            output: Vec::new(),
            error: None,
        };
    }

    let mut runner = match ScriptRunner::new(&script) {
        Ok(runner) => runner,
        Err(error) => {
            return RunResult {
                ok: false,
                diagnostics,
                output: Vec::new(),
                error: Some(error.to_string()),
            };
        }
    };

    let mut output = Vec::new();
    let mut output_bytes = 0;
    let mut answers = replies.into_iter();
    let mut outcome = runner.run();
    let error = loop {
        match outcome {
            Ok(ScriptYield::Finished) => break None,
            Ok(ScriptYield::Host { name, values }) => {
                match perform(&name, &values, &mut output, &mut output_bytes, &mut answers) {
                    Ok(reply) => outcome = runner.resume(reply),
                    Err(message) => break Some(message.to_owned()),
                }
            }
            Err(error) => break Some(error.to_string()),
        }
    };

    RunResult {
        ok: error.is_none(),
        diagnostics,
        output,
        error,
    }
}

/// Performs one host effect: `say`/unknown append to `output`; `ask` prints its
/// prompt and pulls the next scripted answer.
fn perform(
    name: &str,
    values: &[Value],
    output: &mut Vec<String>,
    output_bytes: &mut usize,
    answers: &mut impl Iterator<Item = Value>,
) -> Result<Option<Value>, &'static str> {
    match name {
        "ask" => {
            push_output_values(output, output_bytes, "", values)?;
            Ok(answers.next())
        }
        "say" => {
            push_output_values(output, output_bytes, "", values)?;
            Ok(None)
        }
        other => {
            let prefix = format!("[{other}] ");
            push_output_values(output, output_bytes, &prefix, values)?;
            Ok(None)
        }
    }
}

fn push_output(
    output: &mut Vec<String>,
    output_bytes: &mut usize,
    line: String,
) -> Result<(), &'static str> {
    let total = output_bytes
        .checked_add(line.len())
        .and_then(|bytes| bytes.checked_add(1))
        .ok_or("execution budget exceeded: output size overflow")?;
    if total > MAX_OUTPUT_BYTES {
        return Err("execution budget exceeded: output exceeds 1 MiB");
    }
    *output_bytes = total;
    output.push(line);
    Ok(())
}

fn push_output_values(
    output: &mut Vec<String>,
    output_bytes: &mut usize,
    prefix: &str,
    values: &[Value],
) -> Result<(), &'static str> {
    let limit = MAX_OUTPUT_BYTES
        .checked_sub(*output_bytes)
        .and_then(|remaining| remaining.checked_sub(1))
        .ok_or("execution budget exceeded: output exceeds 1 MiB")?;
    let mut line = String::new();
    push_line_text(&mut line, prefix, limit)?;
    for (index, value) in values.iter().enumerate() {
        if index > 0 {
            push_line_text(&mut line, " ", limit)?;
        }
        value
            .append_to_display(&mut line, limit)
            .map_err(|_| "execution budget exceeded: output exceeds 1 MiB")?;
    }
    push_output(output, output_bytes, line)
}

fn push_line_text(output: &mut String, text: &str, limit: usize) -> Result<(), &'static str> {
    if output
        .len()
        .checked_add(text.len())
        .is_none_or(|length| length > limit)
    {
        return Err("execution budget exceeded: output exceeds 1 MiB");
    }
    output.push_str(text);
    Ok(())
}

/// Parses the caller's JSON reply array into Velin values.
///
/// The entire input is rejected if any item is unsupported. Silently dropping
/// an item would shift every subsequent answer to the wrong `ask` effect.
pub fn parse_replies(replies_json: &str) -> Result<Vec<Value>, ParseRepliesError> {
    if replies_json.len() > MAX_REPLIES_JSON_BYTES {
        return Err(ParseRepliesError::TooLarge);
    }
    let parsed = serde_json::from_str::<Vec<serde_json::Value>>(replies_json)
        .map_err(|error| ParseRepliesError::InvalidJson(error.to_string()))?;
    parsed
        .iter()
        .enumerate()
        .map(|(index, value)| {
            json_to_value(value).ok_or(ParseRepliesError::UnsupportedValue { index: index + 1 })
        })
        .collect()
}

/// Maps a JSON scalar to a Velin [`Value`] (integers, booleans, strings only —
/// Velin has no floats, so a non-integer number is dropped).
fn json_to_value(value: &serde_json::Value) -> Option<Value> {
    match value {
        serde_json::Value::Bool(boolean) => Some(Value::Boolean(*boolean)),
        serde_json::Value::Number(number) => number.as_i64().map(Value::Integer),
        serde_json::Value::String(text) => Some(Value::String(text.clone().into())),
        _ => None,
    }
}

/// Runs the compile + static-check pass, returning flattened diagnostics.
fn diagnose(file: &str, source: &str) -> Vec<WireDiagnostic> {
    match compile(file, source) {
        Ok(script) => check_script(file, &script)
            .iter()
            .map(WireDiagnostic::from)
            .collect(),
        Err(diagnostic) => vec![WireDiagnostic::from(&diagnostic)],
    }
}

#[cfg(test)]
mod tests {
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
            Err(ParseRepliesError::UnsupportedValue { index: 2 })
        );
        assert_eq!(
            parse_replies(&" ".repeat(MAX_REPLIES_JSON_BYTES + 1)),
            Err(ParseRepliesError::TooLarge)
        );
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
            Err(ParseRepliesError::UnsupportedValue { index: 1 })
        ));
        assert!(parse_replies("[]").unwrap().is_empty());
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
}
