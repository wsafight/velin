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
use std::path::Path;
use std::process::ExitCode;
use velin::{
    ARTIFACT_MAGIC, CompiledScript, MAX_ARTIFACT_BYTES, MAX_SOURCE_BYTES, artifact_cache_key,
    artifact_cache_path, check_script, compile, decode_artifact, encode_artifact,
    load_artifact_cache, store_artifact_cache,
};

/// Parsed command line: a verb and the script path it applies to.
enum Command {
    Check { path: String, json: bool },
    Run(String),
    Compile { input: String, output: String },
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
    velin run            <file.velin|.velinc|->  check/run against the reference host
    velin compile        <file.velin> -o <file.velinc>  write a reusable bytecode artifact
    velin --help                              show this help
    velin --version                           show the version

Use `-` to read source from stdin. `run -` is limited to scripts without host replies;
use a file when `ask` must read stdin. JSON output is available for `check` only.";

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
        "compile" => parse_compile_args(args),
        other => Err(format!("unknown subcommand `{other}`")),
    }
}

fn parse_compile_args(mut args: impl Iterator<Item = String>) -> Result<Command, String> {
    let input = args.next().ok_or("`compile` needs an input file")?;
    if input.starts_with('-') && input != "-" {
        return Err(format!("unknown option `{input}` for `compile`"));
    }
    let option = args.next().ok_or("`compile` needs `-o <output>`")?;
    if option != "-o" {
        return Err("`compile` expects `-o <output>`".to_owned());
    }
    let output = args.next().ok_or("`compile` needs an output file")?;
    if output.starts_with('-') && output != "-" {
        return Err(format!("unknown option `{output}` for `compile`"));
    }
    if let Some(extra) = args.next() {
        return Err(format!("unexpected extra argument `{extra}`"));
    }
    Ok(Command::Compile { input, output })
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
        Command::Compile { input, output } => compile_artifact(&input, &output),
        Command::Help | Command::Version => ExitCode::SUCCESS,
    }
}

/// `velin check`: report every diagnostic; fail only on errors.
fn check(path: &str, json: bool) -> ExitCode {
    let bytes = match read_bytes(path, MAX_ARTIFACT_BYTES.max(MAX_SOURCE_BYTES)) {
        Ok(bytes) => bytes,
        Err(message) => {
            if json {
                print_check_json(false, &[], Some(&message));
            } else {
                eprintln!("{message}");
            }
            return ExitCode::from(2);
        }
    };
    if bytes.starts_with(ARTIFACT_MAGIC) {
        return match decode_artifact(&bytes) {
            Ok(artifact) => {
                let file = artifact.source_name();
                let diagnostics = check_script(file, artifact.script());
                let ok = diagnostics.iter().all(|diagnostic| !diagnostic.is_error());
                if json {
                    print_check_json(ok, &diagnostics, None);
                } else {
                    for diagnostic in &diagnostics {
                        eprintln!("{diagnostic}");
                    }
                    if ok {
                        println!("{path}: ok");
                    }
                }
                if ok {
                    ExitCode::SUCCESS
                } else {
                    ExitCode::FAILURE
                }
            }
            Err(error) => {
                if json {
                    print_check_json(false, &[], Some(&error.to_string()));
                } else {
                    eprintln!("{path}: {error}");
                }
                ExitCode::FAILURE
            }
        };
    }
    let file = source_name(path);
    let source = match String::from_utf8(bytes) {
        Ok(source) => source,
        Err(error) => {
            let message = format!("cannot read `{path}`: {error}");
            if json {
                print_check_json(false, &[], Some(&message));
            } else {
                eprintln!("{message}");
            }
            return ExitCode::from(2);
        }
    };

    let script = match compile_source_cached(path, file, &source) {
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
    let bytes = match read_bytes(path, MAX_ARTIFACT_BYTES.max(MAX_SOURCE_BYTES)) {
        Ok(bytes) => bytes,
        Err(message) => {
            eprintln!("{message}");
            return ExitCode::from(2);
        }
    };
    let artifact = bytes
        .starts_with(ARTIFACT_MAGIC)
        .then(|| decode_artifact(&bytes));
    let script = match artifact {
        Some(Ok(artifact)) => {
            let source_name = artifact.source_name().to_owned();
            let script = artifact.into_script();
            let diagnostics = check_script(&source_name, &script);
            if diagnostics.iter().any(velin::Diagnostic::is_error) {
                for diagnostic in diagnostics {
                    eprintln!("{diagnostic}");
                }
                return ExitCode::FAILURE;
            }
            script
        }
        Some(Err(error)) => {
            eprintln!("{path}: {error}");
            return ExitCode::FAILURE;
        }
        None => {
            let file = source_name(path);
            let source = match String::from_utf8(bytes) {
                Ok(source) => source,
                Err(error) => {
                    eprintln!("cannot read `{path}`: {error}");
                    return ExitCode::from(2);
                }
            };
            let script = match compile_source_cached(path, file, &source) {
                Ok(script) => script,
                Err(diagnostic) => {
                    eprintln!("{diagnostic}");
                    return ExitCode::FAILURE;
                }
            };
            // Source runs retain the static-check gate. Artifacts are checked
            // below as well, so cache hits and misses have identical behavior.
            let diagnostics = check_script(file, &script);
            let errors: Vec<_> = diagnostics.iter().filter(|d| d.is_error()).collect();
            if !errors.is_empty() {
                for diagnostic in &errors {
                    eprintln!("{diagnostic}");
                }
                return ExitCode::FAILURE;
            }
            script
        }
    };

    if path == "-"
        && script.program.ops.iter().any(|op| {
            matches!(op, velin::Op::Host(host)
                    if host.bind.is_some()
                        || script.host_name(host.host_id) == Some("ask"))
        })
    {
        eprintln!(
            "`run -` cannot execute scripts that request host replies; use a source file and provide replies on stdin"
        );
        return ExitCode::from(2);
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
fn compile_source_cached(
    path: &str,
    file: &str,
    source: &str,
) -> Result<CompiledScript, velin::Diagnostic> {
    if path == "-" {
        return compile(file, source);
    }
    let source_path = Path::new(path);
    let cache_dir = source_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(".velin-cache");
    let key = artifact_cache_key(
        source.as_bytes(),
        concat!("velin-", env!("CARGO_PKG_VERSION")),
        "default",
        &[],
    );
    let cache_path = artifact_cache_path(&cache_dir, &key);
    if let Ok(Some(artifact)) = load_artifact_cache(&cache_path) {
        return Ok(artifact.into_script());
    }
    let script = compile(file, source)?;
    let _ = store_artifact_cache(&cache_path, file, &script);
    Ok(script)
}
fn compile_artifact(input: &str, output: &str) -> ExitCode {
    if input == "-" {
        eprintln!("`compile` cannot write an artifact when reading source from stdin");
        return ExitCode::from(2);
    }
    let source = match read(input) {
        Ok(source) => source,
        Err(message) => {
            eprintln!("{message}");
            return ExitCode::from(2);
        }
    };
    let script = match compile(input, &source) {
        Ok(script) => script,
        Err(diagnostic) => {
            eprintln!("{diagnostic}");
            return ExitCode::FAILURE;
        }
    };
    let diagnostics = check_script(input, &script);
    if diagnostics.iter().any(velin::Diagnostic::is_error) {
        for diagnostic in diagnostics {
            eprintln!("{diagnostic}");
        }
        return ExitCode::FAILURE;
    }
    let artifact = match encode_artifact(input, &script) {
        Ok(artifact) => artifact,
        Err(error) => {
            eprintln!("cannot encode `{output}`: {error}");
            return ExitCode::FAILURE;
        }
    };
    match write_atomic(output, &artifact) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("cannot write `{output}`: {error}");
            ExitCode::from(2)
        }
    }
}
fn write_atomic(path: &str, bytes: &[u8]) -> io::Result<()> {
    let target = std::path::Path::new(path);
    let parent = target.parent().unwrap_or_else(|| std::path::Path::new("."));
    let name = target
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("artifact");
    let temporary = parent.join(format!(
        ".{name}.{}.{}.tmp",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let result = (|| {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        replace_file(&temporary, target)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result
}
fn replace_file(temporary: &Path, target: &Path) -> io::Result<()> {
    #[cfg(windows)]
    if target.exists() {
        // `std::fs::rename` refuses an existing destination on Windows.
        // Remove it before installing the fully synced temporary file so
        // repeated artifact writes have the same behavior on every platform.
        std::fs::remove_file(target)?;
    }
    std::fs::rename(temporary, target)
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
fn read_bytes(path: &str, limit: usize) -> Result<Vec<u8>, String> {
    if path == "-" {
        let stdin = io::stdin();
        return read_bounded(stdin.lock(), "<stdin>", limit);
    }
    let file =
        std::fs::File::open(path).map_err(|error| format!("cannot read `{path}`: {error}"))?;
    read_bounded(file, path, limit)
}

fn read_bounded(mut input: impl Read, name: &str, limit: usize) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    input
        .by_ref()
        .take((limit + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("cannot read `{name}`: {error}"))?;
    if bytes.len() > limit {
        return Err(format!("cannot read `{name}`: input exceeds {limit} bytes"));
    }
    Ok(bytes)
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
#[path = "main_tests.rs"]
mod tests;
