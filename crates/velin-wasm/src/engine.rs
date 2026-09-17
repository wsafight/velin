//! The Playground engine: `.velin` source in, serializable result out.
//!
//! Kept free of any `wasm_bindgen` attributes so it compiles and unit-tests on
//! the host. The wasm layer in `lib.rs` is a one-line string wrapper over each
//! function here. A *scripted* host makes a run deterministic: `say` appends a
//! line to the output log, and `ask` is answered from a fixed reply list, so
//! the same source + replies always produce the same transcript.

use serde::Serialize;
use std::collections::BTreeMap;
use std::fmt;
use velin::{
    DebugEvent, DebugSession, DebugSnapshot, Diagnostic, MarshallingLimits, ScriptRunner,
    ScriptYield, Value, check_script, compile, debug_script, json_to_value,
};

const MAX_OUTPUT_BYTES: usize = 1024 * 1024;
const MAX_REPLIES_JSON_BYTES: usize = 1024 * 1024;

/// A caller error in the scripted replies supplied to the Playground host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseRepliesError {
    TooLarge,
    InvalidJson(String),
    UnsupportedValue {
        index: usize,
        path: String,
        message: String,
    },
}

impl fmt::Display for ParseRepliesError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooLarge => write!(formatter, "invalid replies: JSON exceeds 1 MiB"),
            Self::InvalidJson(message) => write!(formatter, "invalid replies JSON: {message}"),
            Self::UnsupportedValue {
                index,
                path,
                message,
            } => write!(
                formatter,
                "invalid replies: item {index} at {path}: {message}"
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
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct WireDiagnostic {
    pub severity: String,
    pub line: usize,
    pub column: usize,
    pub message: String,
}

/// Serializable state for the interactive Playground debugger.
#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct DebugResult {
    pub ok: bool,
    pub status: String,
    pub line: usize,
    pub diagnostics: Vec<WireDiagnostic>,
    pub output: Vec<String>,
    pub variables: BTreeMap<String, Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snapshot: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone)]
struct BranchSnapshot {
    machine: DebugSnapshot,
    output: Vec<String>,
    output_bytes: usize,
}

/// Persistent deterministic execution used by the Playground's debug controls.
pub struct InteractiveSession {
    script: velin::CompiledScript,
    debugger: DebugSession,
    output: Vec<String>,
    output_bytes: usize,
    snapshots: BTreeMap<u32, BranchSnapshot>,
    next_snapshot: u32,
    last_line: usize,
    status: &'static str,
}

impl InteractiveSession {
    pub fn new(file: &str, source: &str) -> Result<Self, Box<DebugResult>> {
        let script = compile(file, source).map_err(|diagnostic| {
            Box::new(DebugResult {
                ok: false,
                status: "error".to_owned(),
                line: diagnostic.line,
                diagnostics: vec![WireDiagnostic::from(&diagnostic)],
                output: Vec::new(),
                variables: BTreeMap::new(),
                snapshot: None,
                error: None,
            })
        })?;
        let diagnostics: Vec<_> = check_script(file, &script)
            .iter()
            .map(WireDiagnostic::from)
            .collect();
        if diagnostics
            .iter()
            .any(|diagnostic| diagnostic.severity == "error")
        {
            return Err(Box::new(DebugResult {
                ok: false,
                status: "error".to_owned(),
                line: 0,
                diagnostics,
                output: Vec::new(),
                variables: BTreeMap::new(),
                snapshot: None,
                error: None,
            }));
        }
        let debugger = debug_script(&script).map_err(|error| {
            Box::new(DebugResult {
                ok: false,
                status: "error".to_owned(),
                line: 0,
                diagnostics: diagnostics.clone(),
                output: Vec::new(),
                variables: BTreeMap::new(),
                snapshot: None,
                error: Some(error.to_string()),
            })
        })?;
        Ok(Self {
            script,
            debugger,
            output: Vec::new(),
            output_bytes: 0,
            snapshots: BTreeMap::new(),
            next_snapshot: 1,
            last_line: 1,
            status: "paused",
        })
    }

    pub fn continue_execution(&mut self, replies: Vec<Value>) -> DebugResult {
        self.advance(false, replies)
    }

    pub fn step(&mut self, replies: Vec<Value>) -> DebugResult {
        self.advance(true, replies)
    }

    pub fn snapshot(&mut self) -> DebugResult {
        let id = self.next_snapshot;
        self.next_snapshot = self.next_snapshot.saturating_add(1);
        self.snapshots.insert(
            id,
            BranchSnapshot {
                machine: self.debugger.snapshot(),
                output: self.output.clone(),
                output_bytes: self.output_bytes,
            },
        );
        self.result(true, Some(id), None)
    }

    pub fn restore(&mut self, id: u32) -> DebugResult {
        let Some(snapshot) = self.snapshots.get(&id).cloned() else {
            return self.result(false, None, Some(format!("unknown snapshot {id}")));
        };
        self.debugger.restore(&snapshot.machine);
        self.output = snapshot.output;
        self.output_bytes = snapshot.output_bytes;
        self.last_line = self
            .debugger
            .location()
            .map_or(0, |location| location.line as usize);
        self.status = "paused";
        self.result(true, Some(id), None)
    }

    pub fn state(&self) -> DebugResult {
        self.result(true, None, None)
    }

    fn advance(&mut self, single_step: bool, replies: Vec<Value>) -> DebugResult {
        let event = if single_step {
            self.debugger.step()
        } else {
            self.debugger.continue_execution()
        };
        match event {
            Ok(DebugEvent::Paused { location, .. }) => {
                self.last_line = location.line as usize;
                self.status = "paused";
                self.result(true, None, None)
            }
            Ok(DebugEvent::Host {
                host_id,
                values,
                location,
            }) => {
                self.last_line = location.line as usize;
                let Some(name) = self.script.host_name(host_id).map(str::to_owned) else {
                    return self.result(
                        false,
                        None,
                        Some(format!("bytecode yielded unknown host id {host_id}")),
                    );
                };
                let mut replies = replies.into_iter();
                let reply = match perform(
                    &name,
                    &values,
                    &mut self.output,
                    &mut self.output_bytes,
                    &mut replies,
                ) {
                    Ok(reply) => reply,
                    Err(message) => return self.result(false, None, Some(message.to_owned())),
                };
                if let Err(error) = self.debugger.resume(reply) {
                    return self.result(false, None, Some(error.to_string()));
                }
                self.status = "effect";
                self.result(true, None, None)
            }
            Ok(DebugEvent::Finished) => {
                self.status = "finished";
                self.result(true, None, None)
            }
            Err(error) => self.result(false, None, Some(error.to_string())),
        }
    }

    fn result(&self, ok: bool, snapshot: Option<u32>, error: Option<String>) -> DebugResult {
        DebugResult {
            ok,
            status: if error.is_some() {
                "error".to_owned()
            } else {
                self.status.to_owned()
            },
            line: self.last_line,
            diagnostics: Vec::new(),
            output: self.output.clone(),
            variables: self
                .debugger
                .variables()
                .into_iter()
                .map(|variable| (variable.name, variable.value))
                .collect(),
            snapshot,
            error,
        }
    }
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
            json_to_value(value, MarshallingLimits::default()).map_err(|error| {
                ParseRepliesError::UnsupportedValue {
                    index: index + 1,
                    path: error.path().to_owned(),
                    message: error.message().to_owned(),
                }
            })
        })
        .collect()
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
#[path = "engine_tests.rs"]
mod tests;
