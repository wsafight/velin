//! The line-based reference host that `velin run` drives a script against.
//!
//! Velin's core never names a host command; the CLI supplies concrete meaning
//! for a *reference* vocabulary so a `.velin` file can be run from a terminal:
//!
//! * `say(...)` writes its arguments (space-joined) to stdout.
//! * `ask(...)` writes its prompt, reads one line of stdin, and resumes the
//!   script with that answer (parsed as an integer when it looks like one, a
//!   boolean for `true`/`false`, otherwise the raw string).
//! * any other command is treated as a plain side effect: its name and
//!   arguments are echoed and the script resumes with no value.
//!
//! Keeping this in the CLI (not the library) preserves the core's rule that a
//! host command is opaque; embedders replace this reference vocabulary with
//! their own effect handlers.

use std::io::{self, BufRead, Write};
use velin::{CompiledScript, ScriptRunError, ScriptRunner, ScriptYield, Value};

const MAX_OUTPUT_BYTES: usize = 1024 * 1024;

/// A failure while running a script against the reference host.
#[derive(Debug)]
pub enum RunError {
    /// Shared script initialization, evaluation, or execution-budget failure.
    Script(ScriptRunError),
    /// Reading a reply or writing output failed.
    Io(io::Error),
    /// The reference host exceeded its bounded execution or output budget.
    Budget(&'static str),
}

impl std::fmt::Display for RunError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Script(error) => error.fmt(f),
            Self::Io(error) => write!(f, "i/o error: {error}"),
            Self::Budget(message) => write!(f, "execution budget exceeded: {message}"),
        }
    }
}

impl From<io::Error> for RunError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<ScriptRunError> for RunError {
    fn from(error: ScriptRunError) -> Self {
        Self::Script(error)
    }
}

/// Runs `script` to completion, seeding its `default`s and dispatching every
/// host effect to the reference vocabulary. Effects read from `input` and write
/// to `output` so the loop is fully testable.
///
/// # Errors
/// Returns [`RunError::Script`] if the machine raises an evaluation error, or
/// [`RunError::Io`] if reading a reply or writing output fails.
pub fn run(
    script: &CompiledScript,
    input: &mut impl BufRead,
    output: &mut impl Write,
) -> Result<(), RunError> {
    let mut runner = ScriptRunner::new(script)?;
    let mut outcome = runner.run()?;
    let mut output_bytes = 0;
    loop {
        match outcome {
            ScriptYield::Finished => break,
            ScriptYield::Host { name, values } => {
                let reply = perform(&name, &values, input, output, &mut output_bytes)?;
                outcome = runner.resume(reply)?;
            }
        }
    }
    Ok(())
}

/// Performs one host effect and returns the value (if any) to resume with.
fn perform(
    name: &str,
    values: &[Value],
    input: &mut impl BufRead,
    output: &mut impl Write,
    output_bytes: &mut usize,
) -> Result<Option<Value>, RunError> {
    match name {
        "say" => {
            write_values_bounded(output, "", values, "\n", output_bytes)?;
            Ok(None)
        }
        "ask" => {
            write_values_bounded(output, "", values, " ", output_bytes)?;
            output.flush()?;
            let mut line = String::new();
            let read = input.read_line(&mut line)?;
            if read == 0 {
                // End of input: resume with nothing rather than blocking.
                Ok(None)
            } else {
                Ok(Some(parse_reply(line.trim())))
            }
        }
        other => {
            // An unmodelled command is still a legitimate effect; echo it so a
            // script exercising a custom verb can be run without a bespoke host.
            let prefix = format!("[{other}] ");
            write_values_bounded(output, &prefix, values, "\n", output_bytes)?;
            Ok(None)
        }
    }
}

fn write_bounded(
    output: &mut impl Write,
    text: &str,
    suffix: &str,
    output_bytes: &mut usize,
) -> Result<(), RunError> {
    let added = text
        .len()
        .checked_add(suffix.len())
        .ok_or(RunError::Budget("output size overflow"))?;
    let total = output_bytes
        .checked_add(added)
        .ok_or(RunError::Budget("output size overflow"))?;
    if total > MAX_OUTPUT_BYTES {
        return Err(RunError::Budget("output exceeds 1 MiB"));
    }
    output.write_all(text.as_bytes())?;
    output.write_all(suffix.as_bytes())?;
    *output_bytes = total;
    Ok(())
}

fn write_values_bounded(
    output: &mut impl Write,
    prefix: &str,
    values: &[Value],
    suffix: &str,
    output_bytes: &mut usize,
) -> Result<(), RunError> {
    let limit = MAX_OUTPUT_BYTES
        .checked_sub(*output_bytes)
        .and_then(|remaining| remaining.checked_sub(suffix.len()))
        .ok_or(RunError::Budget("output exceeds 1 MiB"))?;
    let mut text = String::new();
    push_text_bounded(&mut text, prefix, limit)?;
    for (index, value) in values.iter().enumerate() {
        if index > 0 {
            push_text_bounded(&mut text, " ", limit)?;
        }
        value
            .append_to_display(&mut text, limit)
            .map_err(|_| RunError::Budget("output exceeds 1 MiB"))?;
    }
    write_bounded(output, &text, suffix, output_bytes)
}

fn push_text_bounded(output: &mut String, text: &str, limit: usize) -> Result<(), RunError> {
    if output
        .len()
        .checked_add(text.len())
        .is_none_or(|length| length > limit)
    {
        return Err(RunError::Budget("output exceeds 1 MiB"));
    }
    output.push_str(text);
    Ok(())
}

/// Interprets a line of stdin as a Velin [`Value`]: an integer when it parses
/// as one, a boolean for `true`/`false`, otherwise the raw string.
fn parse_reply(text: &str) -> Value {
    if let Ok(number) = text.parse::<i64>() {
        Value::Integer(number)
    } else if text == "true" {
        Value::Boolean(true)
    } else if text == "false" {
        Value::Boolean(false)
    } else {
        Value::String(text.into())
    }
}

#[cfg(test)]
#[path = "host_tests.rs"]
mod tests;
