//! Versioned, bounded bytecode artifacts for cross-process script reuse.

use crate::host::{CompiledScript, HostCheckSite};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use velin_bytecode::{InitialFrame, Op, Pc, Program, ProgramValidationError, ValidatedProgram};
use velin_check::{TypeCheckKind, TypeCheckSite};
use velin_syntax::{DataFootprint, Value};

mod wire;
use wire::validate_check_sites;
use wire::{HostSiteWire, TypeCheckKindWire, TypeSiteWire, decode_wire_value, encode_wire_value};

/// Fixed marker at the start of every `.velinc` file.
pub const ARTIFACT_MAGIC: &[u8; 8] = b"VELINBC\0";
/// Current artifact payload version.
pub const ARTIFACT_VERSION: u16 = 4;
/// Maximum complete artifact size accepted by the decoder.
pub const MAX_ARTIFACT_BYTES: usize = 16 * 1024 * 1024;
const HEADER_BYTES: usize = ARTIFACT_MAGIC.len() + 2 + 8;
const MAX_ARTIFACT_ENTRIES: usize = 100_000;
const MAX_ARTIFACT_TEXT_BYTES: usize = 16 * 1024 * 1024;
const MAX_MACHINE_DATA_VALUES: usize = 100_000;
const MAX_MACHINE_TEXT_BYTES: usize = 16 * 1024 * 1024;

/// A decoded artifact with its source name and runnable script.
#[derive(Debug, Clone)]
pub struct BytecodeArtifact {
    source_name: String,
    script: CompiledScript,
}

impl BytecodeArtifact {
    /// Returns the diagnostic source name embedded in the artifact.
    #[must_use]
    pub fn source_name(&self) -> &str {
        &self.source_name
    }

    /// Returns the decoded runnable script.
    #[must_use]
    pub const fn script(&self) -> &CompiledScript {
        &self.script
    }

    /// Consumes the artifact and returns its runnable script.
    #[must_use]
    pub fn into_script(self) -> CompiledScript {
        self.script
    }
}

/// Failure while encoding or decoding a bounded artifact.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtifactError {
    pub message: String,
}

impl ArtifactError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl std::fmt::Display for ArtifactError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for ArtifactError {}

#[derive(Debug, Serialize, Deserialize)]
struct ArtifactPayload {
    source_name: String,
    program: Program,
    hosts: Vec<String>,
    labels: BTreeMap<String, Pc>,
    defaults: BTreeMap<String, Value>,
    type_sites: Vec<TypeSiteWire>,
    host_sites: Vec<HostSiteWire>,
}

/// Encodes a compiled script into a versioned artifact.
///
/// The payload deliberately contains only stable source metadata and the
/// canonical `Program`; validation proofs and execution caches are rebuilt on
/// load.
///
/// # Errors
/// Returns an error when metadata, data budgets, or the encoded payload exceeds
/// the artifact limits.
pub fn encode_artifact(
    source_name: &str,
    script: &CompiledScript,
) -> Result<Vec<u8>, ArtifactError> {
    validate_source_name(source_name)?;
    validate_metadata(
        &script.program,
        &script.hosts,
        &script.labels,
        &script.defaults,
    )?;
    if script.type_sites.len() > MAX_ARTIFACT_ENTRIES
        || script.host_sites.len() > MAX_ARTIFACT_ENTRIES
    {
        return Err(ArtifactError::new(
            "artifact has too many static check sites",
        ));
    }
    let payload = ArtifactPayload {
        source_name: source_name.to_owned(),
        program: (*script.program).clone(),
        hosts: script.hosts.clone(),
        labels: script.labels.clone(),
        defaults: script.defaults.clone(),
        type_sites: script
            .type_sites
            .iter()
            .map(|site| TypeSiteWire {
                pc: site.pc,
                expression: site.expression.clone(),
                kind: match site.kind {
                    TypeCheckKind::Expression => TypeCheckKindWire::Expression,
                    TypeCheckKind::Condition => TypeCheckKindWire::Condition,
                    TypeCheckKind::Assignment => TypeCheckKindWire::Assignment,
                },
            })
            .collect(),
        host_sites: script
            .host_sites
            .iter()
            .map(|site| HostSiteWire {
                host_id: site.host_id,
                arguments: site.arguments,
                bind: site.bind,
                line: site.line,
            })
            .collect(),
    };
    let value = serde_json::to_value(payload)
        .map_err(|error| ArtifactError::new(format!("cannot encode artifact: {error}")))?;
    let mut payload = Vec::new();
    encode_wire_value(&value, &mut payload)?;
    let total = HEADER_BYTES
        .checked_add(payload.len())
        .ok_or_else(|| ArtifactError::new("artifact size overflow"))?;
    if total > MAX_ARTIFACT_BYTES {
        return Err(ArtifactError::new(format!(
            "artifact exceeds {MAX_ARTIFACT_BYTES} bytes"
        )));
    }
    let payload_len = u64::try_from(payload.len())
        .map_err(|_| ArtifactError::new("artifact payload length overflow"))?;
    let mut output = Vec::with_capacity(total);
    output.extend_from_slice(ARTIFACT_MAGIC);
    output.extend_from_slice(&ARTIFACT_VERSION.to_le_bytes());
    output.extend_from_slice(&payload_len.to_le_bytes());
    output.extend_from_slice(&payload);
    Ok(output)
}

/// Decodes and validates an artifact before returning an executable script.
///
/// # Errors
/// Returns an error for a truncated, oversized, unknown-version, malformed, or
/// semantically invalid artifact.
///
/// # Panics
/// Does not panic for any byte slice; header conversions are guarded by the
/// length check at the start of the function.
#[allow(clippy::too_many_lines)]
pub fn decode_artifact(bytes: &[u8]) -> Result<BytecodeArtifact, ArtifactError> {
    if bytes.len() > MAX_ARTIFACT_BYTES {
        return Err(ArtifactError::new(format!(
            "artifact exceeds {MAX_ARTIFACT_BYTES} bytes"
        )));
    }
    if bytes.len() < HEADER_BYTES {
        return Err(ArtifactError::new("artifact header is truncated"));
    }
    if &bytes[..ARTIFACT_MAGIC.len()] != ARTIFACT_MAGIC {
        return Err(ArtifactError::new("invalid artifact magic"));
    }
    let version_start = ARTIFACT_MAGIC.len();
    let version = u16::from_le_bytes(
        bytes[version_start..version_start + 2]
            .try_into()
            .expect("header length was checked"),
    );
    if version != ARTIFACT_VERSION {
        return Err(ArtifactError::new(format!(
            "unsupported artifact version {version}"
        )));
    }
    let length_start = version_start + 2;
    let payload_len = u64::from_le_bytes(
        bytes[length_start..length_start + 8]
            .try_into()
            .expect("header length was checked"),
    );
    let payload_len = usize::try_from(payload_len)
        .map_err(|_| ArtifactError::new("artifact payload length does not fit in memory"))?;
    let expected = HEADER_BYTES
        .checked_add(payload_len)
        .ok_or_else(|| ArtifactError::new("artifact payload length overflow"))?;
    if expected != bytes.len() {
        return Err(ArtifactError::new(
            "artifact payload length does not match file size",
        ));
    }
    let mut cursor = 0;
    let value = decode_wire_value(&bytes[HEADER_BYTES..], &mut cursor, 0)?;
    if cursor != payload_len {
        return Err(ArtifactError::new("artifact payload has trailing bytes"));
    }
    let payload: ArtifactPayload = serde_json::from_value(value)
        .map_err(|error| ArtifactError::new(format!("cannot decode artifact payload: {error}")))?;
    validate_source_name(&payload.source_name)?;
    validate_metadata(
        &payload.program,
        &payload.hosts,
        &payload.labels,
        &payload.defaults,
    )?;
    validate_check_sites(
        &payload.program,
        &payload.hosts,
        &payload.type_sites,
        &payload.host_sites,
    )?;
    let program = Arc::new(payload.program);
    let validated_program = ValidatedProgram::new(program.clone())
        .map_err(|error| ArtifactError::new(format!("invalid artifact program: {error}")))?;
    let initial_frame = InitialFrame::from_named_values(
        &program.slots,
        payload
            .defaults
            .iter()
            .map(|(name, value)| (name.as_str(), value)),
    )
    .map_err(|error| ArtifactError::new(error.to_string()))?;
    let initial_types = initial_frame
        .values()
        .iter()
        .map(|value| {
            value
                .as_ref()
                .map_or(velin_check::Type::Unknown, Into::into)
        })
        .collect();
    let script = CompiledScript::from_artifact(
        program,
        validated_program,
        payload.hosts,
        payload.labels,
        payload.defaults,
        initial_frame,
        initial_types,
        payload
            .type_sites
            .into_iter()
            .map(|site| TypeCheckSite {
                pc: site.pc,
                expression: site.expression,
                kind: match site.kind {
                    TypeCheckKindWire::Expression => TypeCheckKind::Expression,
                    TypeCheckKindWire::Condition => TypeCheckKind::Condition,
                    TypeCheckKindWire::Assignment => TypeCheckKind::Assignment,
                },
            })
            .collect(),
        payload
            .host_sites
            .into_iter()
            .map(|site| HostCheckSite {
                host_id: site.host_id,
                arguments: site.arguments,
                bind: site.bind,
                line: site.line,
            })
            .collect(),
    );
    Ok(BytecodeArtifact {
        source_name: payload.source_name,
        script,
    })
}

/// Computes a stable cache key independent of paths.
#[must_use]
pub fn artifact_cache_key(
    source: &[u8],
    compiler_semantics: &str,
    optimization_level: &str,
    host_schema: &[u8],
) -> String {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for part in [
        source,
        compiler_semantics.as_bytes(),
        optimization_level.as_bytes(),
        host_schema,
        &ARTIFACT_VERSION.to_le_bytes(),
    ] {
        for byte in u64::try_from(part.len()).unwrap_or(u64::MAX).to_le_bytes() {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x0100_0000_01b3);
        }
        for byte in part {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x0100_0000_01b3);
        }
    }
    format!("{hash:016x}.velinc")
}

#[must_use]
pub fn artifact_cache_path(cache_dir: &Path, key: &str) -> PathBuf {
    cache_dir.join(key)
}

/// Loads an artifact cache entry; malformed entries are cache misses.
///
/// # Errors
/// Returns an error when the cache path cannot be read for reasons other than
/// absence.
pub fn load_artifact_cache(path: &Path) -> Result<Option<BytecodeArtifact>, ArtifactError> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(ArtifactError::new(format!("cannot read cache: {error}"))),
    };
    if let Ok(artifact) = decode_artifact(&bytes) {
        Ok(Some(artifact))
    } else {
        let _ = std::fs::remove_file(path);
        Ok(None)
    }
}

/// Atomically writes a cache entry through a same-directory temporary file.
///
/// # Errors
/// Returns an error if encoding, directory creation, writing, or replacement
/// fails.
pub fn store_artifact_cache(
    path: &Path,
    source_name: &str,
    script: &CompiledScript,
) -> Result<(), ArtifactError> {
    let bytes = encode_artifact(source_name, script)?;
    let parent = path
        .parent()
        .ok_or_else(|| ArtifactError::new("cache path has no parent directory"))?;
    std::fs::create_dir_all(parent)
        .map_err(|error| ArtifactError::new(format!("cannot create cache directory: {error}")))?;
    let temporary = parent.join(format!(
        ".{}.{}.tmp",
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("artifact"),
        std::process::id()
    ));
    std::fs::write(&temporary, &bytes)
        .map_err(|error| ArtifactError::new(format!("cannot write cache: {error}")))?;
    if let Err(error) = replace_file(&temporary, path) {
        let _ = std::fs::remove_file(&temporary);
        return Err(ArtifactError::new(format!("cannot install cache: {error}")));
    }
    Ok(())
}

fn replace_file(temporary: &Path, target: &Path) -> std::io::Result<()> {
    #[cfg(windows)]
    if target.exists() {
        // The standard library cannot request MOVEFILE_REPLACE_EXISTING.
        // Removing the old entry first gives Windows the same repeat-write
        // behavior as Unix while keeping all writes in the target directory.
        std::fs::remove_file(target)?;
    }
    std::fs::rename(temporary, target)
}
fn validate_source_name(source_name: &str) -> Result<(), ArtifactError> {
    if source_name.len() > MAX_ARTIFACT_TEXT_BYTES {
        return Err(ArtifactError::new("artifact source name is too long"));
    }
    Ok(())
}

fn validate_metadata(
    program: &Program,
    hosts: &[String],
    labels: &BTreeMap<String, Pc>,
    defaults: &BTreeMap<String, Value>,
) -> Result<(), ArtifactError> {
    if hosts.len() > MAX_ARTIFACT_ENTRIES {
        return Err(ArtifactError::new("artifact has too many host commands"));
    }
    if labels.len() > MAX_ARTIFACT_ENTRIES {
        return Err(ArtifactError::new("artifact has too many labels"));
    }
    if defaults.len() > program.slots.len() {
        return Err(ArtifactError::new("artifact has too many defaults"));
    }
    let mut text_bytes = 0usize;
    for name in hosts {
        text_bytes = checked_text(text_bytes, name.len())?;
    }
    for (name, target) in labels {
        if usize::try_from(*target).map_or(true, |target| target > program.ops.len()) {
            return Err(ArtifactError::new(format!(
                "label `{name}` points outside the program"
            )));
        }
        text_bytes = checked_text(text_bytes, name.len())?;
    }
    let mut total = DataFootprint::default();
    for (name, value) in defaults {
        if program.slots.get(name).is_none() {
            return Err(ArtifactError::new(format!(
                "default `{name}` does not name a program slot"
            )));
        }
        text_bytes = checked_text(text_bytes, name.len())?;
        let metrics = value
            .data_metrics()
            .map_err(|error| ArtifactError::new(format!("default `{name}` is invalid: {error}")))?;
        total.values = total
            .values
            .checked_add(metrics.footprint.values)
            .ok_or_else(|| ArtifactError::new("default value count overflow"))?;
        total.text_bytes = total
            .text_bytes
            .checked_add(metrics.footprint.text_bytes)
            .ok_or_else(|| ArtifactError::new("default text size overflow"))?;
    }
    if total.values > MAX_MACHINE_DATA_VALUES {
        return Err(ArtifactError::new("defaults exceed machine value budget"));
    }
    if total.text_bytes > MAX_MACHINE_TEXT_BYTES {
        return Err(ArtifactError::new("defaults exceed machine text budget"));
    }
    for op in &program.ops {
        if let Op::Host(host) = op
            && usize::try_from(host.host_id).map_or(true, |id| id >= hosts.len())
        {
            return Err(ArtifactError::new("program references an unknown host id"));
        }
    }
    Ok(())
}

fn checked_text(current: usize, added: usize) -> Result<usize, ArtifactError> {
    let total = current
        .checked_add(added)
        .ok_or_else(|| ArtifactError::new("artifact text size overflow"))?;
    if total > MAX_ARTIFACT_TEXT_BYTES {
        return Err(ArtifactError::new("artifact metadata text exceeds 16 MiB"));
    }
    Ok(total)
}

impl From<ProgramValidationError> for ArtifactError {
    fn from(error: ProgramValidationError) -> Self {
        Self::new(format!("invalid artifact program: {error}"))
    }
}

#[cfg(test)]
#[path = "artifact_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "artifact_coverage_tests.rs"]
mod coverage_tests;
