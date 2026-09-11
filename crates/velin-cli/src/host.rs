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
use velin::{CompiledScript, EvalError, Machine, ProgramValidationError, Value, Yield};

const MAX_OUTPUT_BYTES: usize = 1024 * 1024;

/// A failure while running a script against the reference host.
#[derive(Debug)]
pub enum RunError {
    /// The virtual machine raised an evaluation error (e.g. a type mismatch).
    Eval(EvalError),
    /// The compiled or externally supplied bytecode failed validation.
    Program(ProgramValidationError),
    /// Reading a reply or writing output failed.
    Io(io::Error),
    /// The reference host exceeded its bounded execution or output budget.
    Budget(&'static str),
}

impl std::fmt::Display for RunError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Eval(error) => {
                write!(f, "runtime error at line {}: {}", error.line, error.message)
            }
            Self::Program(error) => write!(f, "invalid bytecode: {error}"),
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

/// Runs `script` to completion, seeding its `default`s and dispatching every
/// host effect to the reference vocabulary. Effects read from `input` and write
/// to `output` so the loop is fully testable.
///
/// # Errors
/// Returns [`RunError::Eval`] if the machine raises an evaluation error, or
/// [`RunError::Io`] if reading a reply or writing output fails.
pub fn run(
    script: &CompiledScript,
    input: &mut impl BufRead,
    output: &mut impl Write,
) -> Result<(), RunError> {
    const MAX_HOST_EFFECTS: usize = 1_000;
    let mut machine = Machine::new(script.program.clone()).map_err(RunError::Program)?;
    for (name, value) in &script.defaults {
        machine.set_variable(name, value.clone());
    }

    let mut outcome = machine.run().map_err(RunError::Eval)?;
    let mut host_effects = 0;
    let mut output_bytes = 0;
    loop {
        match outcome {
            Yield::Finished => break,
            Yield::Host { host_id, values } => {
                host_effects += 1;
                if host_effects > MAX_HOST_EFFECTS {
                    return Err(RunError::Budget("too many host effects"));
                }
                let name = script.host_name(host_id).unwrap_or("<unknown>");
                let reply = perform(name, &values, input, output, &mut output_bytes)?;
                outcome = machine.resume(reply).map_err(RunError::Eval)?;
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
        Value::String(text.to_owned())
    }
}

#[cfg(test)]
mod tests {
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
        assert!(matches!(error, RunError::Budget("too many host effects")));
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
        let eval = RunError::Eval(velin::EvalError::new(4, "boom"));
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
        assert!(matches!(error, RunError::Eval(_)));
        assert!(String::from_utf8(output).unwrap().contains("pick:"));

        let compounds = run_capture("perform say(list(1, true), record(\"a\", \"b\"))\n", "");
        assert_eq!(compounds, "[1, true] {a: b}\n");

        let script = compile("test.velin", "set x = 1 / 0\n").expect("compiles");
        let mut input = Cursor::new(String::new());
        let mut output = Vec::new();
        let error = run(&script, &mut input, &mut output).unwrap_err();
        assert!(matches!(error, RunError::Eval(_)));
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
    }
}
