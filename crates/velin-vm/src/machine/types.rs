use velin_syntax::Value;

/// Bounded execution counters suitable for an offline profile-guided compile.
/// Only validated program-counter IDs are recorded; values and host payloads
/// never enter the profile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutionProfile {
    op_hits: Box<[u64]>,
    enabled: bool,
}

impl ExecutionProfile {
    pub(super) fn new(op_count: usize) -> Self {
        Self {
            op_hits: vec![0; op_count].into_boxed_slice(),
            enabled: true,
        }
    }

    pub(super) fn disabled() -> Self {
        Self {
            op_hits: Box::new([]),
            enabled: false,
        }
    }

    pub(super) fn record(&mut self, pc: usize) {
        if !self.enabled {
            return;
        }
        if let Some(hits) = self.op_hits.get_mut(pc) {
            *hits = hits.saturating_add(1);
        }
    }

    pub(super) fn reset(&mut self) {
        self.op_hits.fill(0);
    }

    /// Returns whether this profile records instruction hits.
    #[must_use]
    pub const fn enabled(&self) -> bool {
        self.enabled
    }

    /// Returns per-op hit counters in program-counter order.
    #[must_use]
    pub fn op_hits(&self) -> &[u64] {
        &self.op_hits
    }

    /// Returns anonymous hot op IDs at or above `threshold` hits.
    #[must_use]
    pub fn hot_ops(&self, threshold: u64) -> Vec<(u32, u64)> {
        self.op_hits
            .iter()
            .enumerate()
            .filter_map(|(pc, hits)| {
                (*hits >= threshold)
                    .then(|| Some((u32::try_from(pc).ok()?, *hits)))
                    .flatten()
            })
            .collect()
    }

    /// Merges counters from another profile with the same program width.
    ///
    /// # Errors
    /// Returns an error if the profiles refer to different program widths.
    pub fn merge(&mut self, other: &Self) -> Result<(), &'static str> {
        if self.op_hits.len() != other.op_hits.len() {
            return Err("execution profiles refer to different program widths");
        }
        for (left, right) in self.op_hits.iter_mut().zip(&other.op_hits) {
            *left = left.saturating_add(*right);
        }
        Ok(())
    }
}

/// Why the machine stopped running.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Yield {
    /// A host effect must be performed. `values` are the evaluated arguments.
    /// The embedder handles `host_id` and calls [`Machine::resume`](super::Machine::resume).
    Host { host_id: u32, values: Vec<Value> },
    /// The program halted normally.
    Finished,
}

/// Result of the low-allocation host execution path.
///
/// Single-argument host calls are returned as [`FastYield::HostOne`] so a
/// caller can avoid allocating a one-element `Vec<Value>`. The machine's
/// pending-host state is identical to [`Yield::Host`] and can be resumed with
/// [`Machine::resume`](super::Machine::resume).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FastYield {
    HostOne { host_id: u32, value: Value },
    Host { host_id: u32, values: Vec<Value> },
    Finished,
}

impl FastYield {
    /// Converts this result to the ordinary allocating yield representation.
    #[must_use]
    pub fn into_yield(self) -> Yield {
        match self {
            Self::HostOne { host_id, value } => Yield::Host {
                host_id,
                values: vec![value],
            },
            Self::Host { host_id, values } => Yield::Host { host_id, values },
            Self::Finished => Yield::Finished,
        }
    }
}

/// A side-effect-only host command collected by [`Machine::run_effect_batch`](super::Machine::run_effect_batch).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostEffect {
    /// The opaque host command identifier supplied by the compiled program.
    pub host_id: u32,
    /// Evaluated arguments owned by the host until it finishes the effect.
    pub values: Vec<Value>,
}

/// Why a host-provided variable could not be installed in a machine frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SetVariableError {
    UnknownVariable(String),
    UnknownSlot(u32),
    InvalidValue(&'static str),
    StateBudget(&'static str),
}

impl std::fmt::Display for SetVariableError {
    #[inline(never)]
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownVariable(name) => write!(formatter, "unknown variable `{name}`"),
            Self::UnknownSlot(slot) => write!(formatter, "unknown slot `{slot}`"),
            Self::InvalidValue(message) | Self::StateBudget(message) => {
                formatter.write_str(message)
            }
        }
    }
}

impl std::error::Error for SetVariableError {}
