use std::process::ExitCode;

pub(super) fn parse_args(args: impl Iterator<Item = String>) -> Result<(String, bool), String> {
    let mut path = None;
    let mut check = false;
    for argument in args {
        if argument == "--check" {
            if check {
                return Err("`--check` was provided more than once".to_owned());
            }
            check = true;
        } else if argument.starts_with('-') && argument != "-" {
            return Err(format!("unknown option `{argument}` for `fmt`"));
        } else if path.replace(argument).is_some() {
            return Err("`fmt` accepts exactly one file path".to_owned());
        }
    }
    Ok((path.ok_or("`fmt` needs a file path")?, check))
}

pub(super) fn run(path: &str, check_only: bool) -> ExitCode {
    let source = match super::read(path) {
        Ok(source) => source,
        Err(message) => {
            eprintln!("{message}");
            return ExitCode::from(2);
        }
    };
    let formatted = match velin::format_source(&source) {
        Ok(formatted) => formatted,
        Err(error) => {
            eprintln!("{}", error.into_diagnostic(super::source_name(path)));
            return ExitCode::FAILURE;
        }
    };
    if check_only {
        if formatted == source {
            return ExitCode::SUCCESS;
        }
        eprintln!("{path}: not formatted");
        return ExitCode::FAILURE;
    }
    if path == "-" {
        print!("{formatted}");
        return ExitCode::SUCCESS;
    }
    if formatted == source {
        return ExitCode::SUCCESS;
    }
    match super::write_atomic(path, formatted.as_bytes()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("cannot write `{path}`: {error}");
            ExitCode::from(2)
        }
    }
}
