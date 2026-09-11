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
    Check(String),
    Run(String),
}

fn main() -> ExitCode {
    match parse_args(std::env::args().skip(1)) {
        Ok(command) => run(command),
        Err(message) => {
            eprintln!("{message}");
            eprintln!("\n{USAGE}");
            ExitCode::from(2)
        }
    }
}

const USAGE: &str = "\
usage:
    velin check <file.velin>    parse + static-check a script
    velin run   <file.velin>    check, then run against the reference host";

/// Parses `argv` (already past the program name) into a [`Command`].
fn parse_args(mut args: impl Iterator<Item = String>) -> Result<Command, String> {
    let verb = args.next().ok_or("missing subcommand")?;
    if verb == "--help" || verb == "-h" || verb == "help" {
        return Err("showing help".to_owned());
    }
    let path = args
        .next()
        .ok_or_else(|| format!("`{verb}` needs a file path"))?;
    if let Some(extra) = args.next() {
        return Err(format!("unexpected extra argument `{extra}`"));
    }
    match verb.as_str() {
        "check" => Ok(Command::Check(path)),
        "run" => Ok(Command::Run(path)),
        other => Err(format!("unknown subcommand `{other}`")),
    }
}

/// Executes a parsed command, mapping the outcome to a process exit code.
fn run(command: Command) -> ExitCode {
    match command {
        Command::Check(path) => check(&path),
        Command::Run(path) => execute(&path),
    }
}

/// `velin check`: report every diagnostic; fail only on errors.
fn check(path: &str) -> ExitCode {
    let source = match read(path) {
        Ok(source) => source,
        Err(code) => return code,
    };

    let script = match compile(path, &source) {
        Ok(script) => script,
        Err(diagnostic) => {
            eprintln!("{diagnostic}");
            return ExitCode::FAILURE;
        }
    };

    let diagnostics = check_script(path, &script);
    for diagnostic in &diagnostics {
        eprintln!("{diagnostic}");
    }
    if diagnostics.iter().any(velin::Diagnostic::is_error) {
        ExitCode::FAILURE
    } else {
        if diagnostics.is_empty() {
            println!("{path}: ok");
        }
        ExitCode::SUCCESS
    }
}

/// `velin run`: check for errors, then drive the reference host.
fn execute(path: &str) -> ExitCode {
    let source = match read(path) {
        Ok(source) => source,
        Err(code) => return code,
    };

    let script = match compile(path, &source) {
        Ok(script) => script,
        Err(diagnostic) => {
            eprintln!("{diagnostic}");
            return ExitCode::FAILURE;
        }
    };

    // A definite-assignment or type error would run into undefined behaviour at
    // the host boundary, so refuse to run a script that fails the checks.
    let diagnostics = check_script(path, &script);
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

/// Reads a script file, mapping an I/O failure to a `2` exit code.
fn read(path: &str) -> Result<String, ExitCode> {
    let file = std::fs::File::open(path).map_err(|error| {
        eprintln!("cannot read `{path}`: {error}");
        ExitCode::from(2)
    })?;
    let mut bytes = Vec::new();
    file.take((MAX_SOURCE_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|error| {
            eprintln!("cannot read `{path}`: {error}");
            ExitCode::from(2)
        })?;
    if bytes.len() > MAX_SOURCE_BYTES {
        eprintln!("cannot read `{path}`: source exceeds 1 MiB");
        return Err(ExitCode::from(2));
    }
    String::from_utf8(bytes).map_err(|error| {
        eprintln!("cannot read `{path}`: {error}");
        ExitCode::from(2)
    })
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
        match parse_args(["check".into(), "a.velin".into()].into_iter()) {
            Ok(Command::Check(path)) => assert_eq!(path, "a.velin"),
            Ok(Command::Run(_)) => panic!("expected check"),
            Err(message) => panic!("expected check, got error {message}"),
        }
        match parse_args(["run".into(), "b.velin".into()].into_iter()) {
            Ok(Command::Run(path)) => assert_eq!(path, "b.velin"),
            Ok(Command::Check(_)) => panic!("expected run"),
            Err(message) => panic!("expected run, got error {message}"),
        }
        assert!(parse_args(std::iter::empty()).is_err());
        assert!(parse_args(["--help".into()].into_iter()).is_err());
        assert!(parse_args(["-h".into()].into_iter()).is_err());
        assert!(parse_args(["help".into()].into_iter()).is_err());
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

        let _ = run(Command::Check(ok.clone()));
        let _ = run(Command::Run(ok));
        let _ = check(&parse_error);
        let _ = execute(&parse_error);
        let _ = check(&check_error);
        let _ = execute(&check_error);
        let _ = execute(&runtime_error);
        let _ = check("/no/such/velin-file.velin");
        let _ = execute("/no/such/velin-file.velin");
        let _ = read("/no/such/velin-file.velin");
    }
}
