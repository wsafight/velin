//! `velin` — the command-line front-end for the Velin language.
//!
//! Two subcommands, both taking a single `.velin` file path:
//!
//! * `velin check <file>` — parse, lower, and statically check the script,
//!   printing every diagnostic. Exit code 0 when there are no *errors*
//!   (warnings alone still pass), 1 otherwise.
//! * `velin run <file>` — check first (compile errors abort), then execute the
//!   script against the line-based reference host in [`host`]: `say` prints,
//!   `ask` reads a line of stdin, other commands echo.
//!
//! The compiler never invents a host command, so the runner's vocabulary lives
//! entirely in `host.rs`; the CLI is a thin shell around the `velin` library.

mod host;

use std::io::{self, Read, Write};
use std::process::ExitCode;
use velin::{MAX_SOURCE_BYTES, check_script, compile};

/// Parsed command line: a verb and the script path it applies to.
enum Command {
    Check { path: String, json: bool },
    Run(String),
    Help,
    Version,
}

fn main() -> ExitCode {
    match parse_args(std::env::args().skip(1)) {
        Ok(Command::Help) => {
            println!("{USAGE}");
            ExitCode::SUCCESS
        }
        Ok(Command::Version) => {
            println!("velin {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        Ok(command) => run(command),
        Err(message) => {
            eprintln!("{message}");
            eprintln!("\n{USAGE}");
            ExitCode::from(2)
        }
    }
}

const USAGE: &str = "\
Velin deterministic scripting language

usage:
    velin check [--json] <file.velin|->    parse + static-check a script
    velin run            <file.velin|->    check, then run against the reference host
    velin --help                              show this help
    velin --version                           show the version

Use `-` to read source from stdin. JSON output is available for `check` only.";

/// Parses `argv` (already past the program name) into a [`Command`].
fn parse_args(mut args: impl Iterator<Item = String>) -> Result<Command, String> {
    let verb = args.next().ok_or("missing subcommand")?;
    if verb == "--help" || verb == "-h" || verb == "help" {
        return ensure_no_extra(args, Command::Help);
    }
    if verb == "--version" || verb == "-V" {
        return ensure_no_extra(args, Command::Version);
    }
    match verb.as_str() {
        "check" => parse_check_args(args),
        "run" => parse_path_arg("run", args).map(Command::Run),
        other => Err(format!("unknown subcommand `{other}`")),
    }
}

fn ensure_no_extra(
    mut args: impl Iterator<Item = String>,
    command: Command,
) -> Result<Command, String> {
    if let Some(extra) = args.next() {
        Err(format!("unexpected extra argument `{extra}`"))
    } else {
        Ok(command)
    }
}

fn parse_check_args(args: impl Iterator<Item = String>) -> Result<Command, String> {
    let mut path = None;
    let mut json = false;
    for argument in args {
        if argument == "--json" {
            if json {
                return Err("`--json` was provided more than once".to_owned());
            }
            json = true;
        } else if argument.starts_with('-') && argument != "-" {
            return Err(format!("unknown option `{argument}` for `check`"));
        } else if path.replace(argument).is_some() {
            return Err("`check` accepts exactly one file path".to_owned());
        }
    }
    let path = path.ok_or("`check` needs a file path")?;
    Ok(Command::Check { path, json })
}

fn parse_path_arg(verb: &str, mut args: impl Iterator<Item = String>) -> Result<String, String> {
    let path = args
        .next()
        .ok_or_else(|| format!("`{verb}` needs a file path"))?;
    if path.starts_with('-') && path != "-" {
        return Err(format!("unknown option `{path}` for `{verb}`"));
    }
    if let Some(extra) = args.next() {
        return Err(format!("unexpected extra argument `{extra}`"));
    }
    Ok(path)
}

/// Executes a parsed command, mapping the outcome to a process exit code.
fn run(command: Command) -> ExitCode {
    match command {
        Command::Check { path, json } => check(&path, json),
        Command::Run(path) => execute(&path),
        Command::Help | Command::Version => ExitCode::SUCCESS,
    }
}

/// `velin check`: report every diagnostic; fail only on errors.
fn check(path: &str, json: bool) -> ExitCode {
    let file = source_name(path);
    let source = match read(path) {
        Ok(source) => source,
        Err(message) => {
            if json {
                print_check_json(false, &[], Some(&message));
            } else {
                eprintln!("{message}");
            }
            return ExitCode::from(2);
        }
    };

    let script = match compile(file, &source) {
        Ok(script) => script,
        Err(diagnostic) => {
            if json {
                print_check_json(false, &[diagnostic], None);
            } else {
                eprintln!("{diagnostic}");
            }
            return ExitCode::FAILURE;
        }
    };

    let diagnostics = check_script(file, &script);
    let ok = diagnostics.iter().all(|diagnostic| !diagnostic.is_error());
    if json {
        print_check_json(ok, &diagnostics, None);
    } else {
        for diagnostic in &diagnostics {
            eprintln!("{diagnostic}");
        }
        if diagnostics.is_empty() {
            println!("{file}: ok");
        }
    }
    if ok {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

fn print_check_json(ok: bool, diagnostics: &[velin::Diagnostic], error: Option<&str>) {
    let result = serde_json::json!({
        "ok": ok,
        "diagnostics": diagnostics,
        "error": error,
    });
    println!("{result}");
}

/// `velin run`: check for errors, then drive the reference host.
fn execute(path: &str) -> ExitCode {
    let file = source_name(path);
    let source = match read(path) {
        Ok(source) => source,
        Err(message) => {
            eprintln!("{message}");
            return ExitCode::from(2);
        }
    };

    let script = match compile(file, &source) {
        Ok(script) => script,
        Err(diagnostic) => {
            eprintln!("{diagnostic}");
            return ExitCode::FAILURE;
        }
    };

    // A definite-assignment or type error would run into undefined behaviour at
    // the host boundary, so refuse to run a script that fails the checks.
    let diagnostics = check_script(file, &script);
    let errors: Vec<_> = diagnostics.iter().filter(|d| d.is_error()).collect();
    if !errors.is_empty() {
        for diagnostic in &errors {
            eprintln!("{diagnostic}");
        }
        return ExitCode::FAILURE;
    }

    let stdin = io::stdin();
    let mut input = stdin.lock();
    let stdout = io::stdout();
    let mut output = stdout.lock();
    match host::run(&script, &mut input, &mut output) {
        Ok(()) => {
            let _ = output.flush();
            ExitCode::SUCCESS
        }
        Err(error) => {
            let _ = output.flush();
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}

/// Reads a bounded UTF-8 source file, or stdin when `path` is `-`.
fn read(path: &str) -> Result<String, String> {
    if path == "-" {
        let stdin = io::stdin();
        return read_source(stdin.lock(), "<stdin>");
    }
    let file =
        std::fs::File::open(path).map_err(|error| format!("cannot read `{path}`: {error}"))?;
    read_source(file, path)
}

fn read_source(mut input: impl Read, name: &str) -> Result<String, String> {
    let mut bytes = Vec::new();
    input
        .by_ref()
        .take((MAX_SOURCE_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("cannot read `{name}`: {error}"))?;
    if bytes.len() > MAX_SOURCE_BYTES {
        return Err(format!("cannot read `{name}`: source exceeds 1 MiB"));
    }
    String::from_utf8(bytes).map_err(|error| format!("cannot read `{name}`: {error}"))
}

fn source_name(path: &str) -> &str {
    if path == "-" { "<stdin>" } else { path }
}

#[cfg(test)]
mod tests {
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
            Ok(Command::Run(_) | Command::Help | Command::Version) => panic!("expected check"),
            Err(message) => panic!("expected check, got error {message}"),
        }
        match parse_args(["run".into(), "b.velin".into()].into_iter()) {
            Ok(Command::Run(path)) => assert_eq!(path, "b.velin"),
            Ok(Command::Check { .. } | Command::Help | Command::Version) => panic!("expected run"),
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
    }
}
