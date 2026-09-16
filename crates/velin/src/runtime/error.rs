use super::ScriptRunError;

impl std::fmt::Display for ScriptRunError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Program(error) => write!(formatter, "invalid bytecode: {error}"),
            Self::InitialValue { name, source } => {
                write!(formatter, "cannot initialize `{name}`: {source}")
            }
            Self::Evaluation(error) => write!(
                formatter,
                "runtime error at line {}: {}",
                error.line, error.message
            ),
            Self::FuelExhausted { limit, immediate } => write!(
                formatter,
                "execution fuel exhausted ({} limit {limit})",
                if *immediate {
                    "immediate"
                } else {
                    "cumulative"
                }
            ),
            Self::Cancelled => formatter.write_str("execution cancelled by host"),
            Self::HostEffectsExceeded { limit } => write!(
                formatter,
                "execution budget exceeded: too many host effects (limit {limit})"
            ),
            Self::HostContract(message) => write!(formatter, "host contract error: {message}"),
        }
    }
}

impl std::error::Error for ScriptRunError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Program(error) => Some(error),
            Self::InitialValue { source, .. } => Some(source),
            Self::Evaluation(error) => Some(error),
            Self::FuelExhausted { .. }
            | Self::Cancelled
            | Self::HostEffectsExceeded { .. }
            | Self::HostContract(_) => None,
        }
    }
}
