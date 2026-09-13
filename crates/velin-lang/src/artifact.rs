//! Versioned, bounded bytecode artifacts for cross-process script reuse.

use crate::host::CompiledScript;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value as JsonValue};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use velin_bytecode::{InitialFrame, Op, Pc, Program, ProgramValidationError, ValidatedProgram};
use velin_syntax::{DataFootprint, Value};

/// Fixed marker at the start of every `.velinc` file.
pub const ARTIFACT_MAGIC: &[u8; 8] = b"VELINBC\0";
/// Current artifact payload version.
pub const ARTIFACT_VERSION: u16 = 3;
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
    let payload = ArtifactPayload {
        source_name: source_name.to_owned(),
        program: (*script.program).clone(),
        hosts: script.hosts.clone(),
        labels: script.labels.clone(),
        defaults: script.defaults.clone(),
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
    );
    Ok(BytecodeArtifact {
        source_name: payload.source_name,
        script,
    })
}

/// Computes a stable cache key for source and every input that can change the
/// resulting execution unit. The key is deliberately independent of paths.
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

/// Returns the path used for one cache key without creating the directory.
#[must_use]
pub fn artifact_cache_path(cache_dir: &Path, key: &str) -> PathBuf {
    cache_dir.join(key)
}

/// Loads an artifact cache entry. A missing or malformed entry is a cache miss
/// so callers can safely recompile; only filesystem failures are reported.
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
/// Returns an error when encoding fails, the cache directory cannot be
/// created, the temporary file cannot be written, or the rename fails.
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
    if let Err(error) = std::fs::rename(&temporary, path) {
        let _ = std::fs::remove_file(&temporary);
        return Err(ArtifactError::new(format!("cannot install cache: {error}")));
    }
    Ok(())
}

const MAX_WIRE_DEPTH: usize = 64;

fn encode_wire_value(value: &JsonValue, output: &mut Vec<u8>) -> Result<(), ArtifactError> {
    match value {
        JsonValue::Null => output.push(0),
        JsonValue::Bool(false) => output.push(1),
        JsonValue::Bool(true) => output.push(2),
        JsonValue::Number(number) => {
            let number = number
                .as_i64()
                .ok_or_else(|| ArtifactError::new("artifact contains a non-integer number"))?;
            output.push(3);
            output.extend_from_slice(&number.to_le_bytes());
        }
        JsonValue::String(text) => {
            output.push(4);
            write_wire_bytes(text.as_bytes(), output)?;
        }
        JsonValue::Array(items) => {
            output.push(5);
            write_wire_len(items.len(), output)?;
            for item in items {
                encode_wire_value(item, output)?;
            }
        }
        JsonValue::Object(fields) => {
            output.push(6);
            write_wire_len(fields.len(), output)?;
            for (key, value) in fields {
                write_wire_bytes(key.as_bytes(), output)?;
                encode_wire_value(value, output)?;
            }
        }
    }
    if output.len() > MAX_ARTIFACT_BYTES {
        return Err(ArtifactError::new(format!(
            "artifact exceeds {MAX_ARTIFACT_BYTES} bytes"
        )));
    }
    Ok(())
}

fn write_wire_len(length: usize, output: &mut Vec<u8>) -> Result<(), ArtifactError> {
    let length =
        u32::try_from(length).map_err(|_| ArtifactError::new("artifact count overflow"))?;
    output.extend_from_slice(&length.to_le_bytes());
    Ok(())
}

fn write_wire_bytes(bytes: &[u8], output: &mut Vec<u8>) -> Result<(), ArtifactError> {
    write_wire_len(bytes.len(), output)?;
    output.extend_from_slice(bytes);
    Ok(())
}

fn decode_wire_value(
    input: &[u8],
    cursor: &mut usize,
    depth: usize,
) -> Result<JsonValue, ArtifactError> {
    if depth > MAX_WIRE_DEPTH {
        return Err(ArtifactError::new("artifact nesting exceeds limit"));
    }
    let tag = read_wire_byte(input, cursor)?;
    match tag {
        0 => Ok(JsonValue::Null),
        1 => Ok(JsonValue::Bool(false)),
        2 => Ok(JsonValue::Bool(true)),
        3 => {
            let bytes = read_wire_slice(input, cursor, 8)?;
            let number = i64::from_le_bytes(bytes.try_into().expect("wire length is eight"));
            Ok(JsonValue::Number(number.into()))
        }
        4 => Ok(JsonValue::String(read_wire_string(input, cursor)?)),
        5 => {
            let count = read_wire_len(input, cursor)?;
            let mut items = Vec::with_capacity(count.min(1_024));
            for _ in 0..count {
                items.push(decode_wire_value(input, cursor, depth + 1)?);
            }
            Ok(JsonValue::Array(items))
        }
        6 => {
            let count = read_wire_len(input, cursor)?;
            let mut fields = Map::new();
            for _ in 0..count {
                let key = read_wire_string(input, cursor)?;
                if fields
                    .insert(key, decode_wire_value(input, cursor, depth + 1)?)
                    .is_some()
                {
                    return Err(ArtifactError::new(
                        "artifact contains duplicate object keys",
                    ));
                }
            }
            Ok(JsonValue::Object(fields))
        }
        _ => Err(ArtifactError::new("artifact contains an unknown wire tag")),
    }
}

fn read_wire_byte(input: &[u8], cursor: &mut usize) -> Result<u8, ArtifactError> {
    let byte = *input
        .get(*cursor)
        .ok_or_else(|| ArtifactError::new("artifact payload is truncated"))?;
    *cursor += 1;
    Ok(byte)
}

fn read_wire_len(input: &[u8], cursor: &mut usize) -> Result<usize, ArtifactError> {
    let bytes = read_wire_slice(input, cursor, 4)?;
    let length = u32::from_le_bytes(bytes.try_into().expect("wire length is four"));
    let length =
        usize::try_from(length).map_err(|_| ArtifactError::new("artifact length overflow"))?;
    if length > MAX_ARTIFACT_BYTES {
        return Err(ArtifactError::new("artifact length exceeds limit"));
    }
    Ok(length)
}

fn read_wire_string(input: &[u8], cursor: &mut usize) -> Result<String, ArtifactError> {
    let length = read_wire_len(input, cursor)?;
    let bytes = read_wire_slice(input, cursor, length)?;
    String::from_utf8(bytes.to_vec())
        .map_err(|_| ArtifactError::new("artifact contains invalid UTF-8"))
}

fn read_wire_slice<'a>(
    input: &'a [u8],
    cursor: &mut usize,
    length: usize,
) -> Result<&'a [u8], ArtifactError> {
    let end = cursor
        .checked_add(length)
        .ok_or_else(|| ArtifactError::new("artifact payload length overflow"))?;
    let bytes = input
        .get(*cursor..end)
        .ok_or_else(|| ArtifactError::new("artifact payload is truncated"))?;
    *cursor = end;
    Ok(bytes)
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
mod tests {
    use super::*;
    use crate::compile;

    #[test]
    fn artifact_round_trip_keeps_runtime_metadata() {
        let script = compile("story.velin", "default hp = 3\nperform say(hp)\n").unwrap();
        let bytes = encode_artifact("story.velin", &script).unwrap();
        assert!(bytes.starts_with(ARTIFACT_MAGIC));
        assert_eq!(bytes[HEADER_BYTES], 6, "payload uses the binary object tag");
        let artifact = decode_artifact(&bytes).unwrap();
        assert_eq!(artifact.source_name(), "story.velin");
        assert_eq!(artifact.script().hosts, script.hosts);
        assert_eq!(artifact.script().defaults, script.defaults);
        assert_eq!(artifact.script().labels, script.labels);
        assert_eq!(artifact.script().program, script.program);
    }

    #[test]
    fn artifact_header_rejects_truncation_versions_and_length_mismatches() {
        let script = compile("test.velin", "set x = 1\n").unwrap();
        let bytes = encode_artifact("test.velin", &script).unwrap();
        assert!(decode_artifact(&bytes[..HEADER_BYTES - 1]).is_err());

        let mut previous_version = bytes.clone();
        previous_version[ARTIFACT_MAGIC.len()..ARTIFACT_MAGIC.len() + 2]
            .copy_from_slice(&2_u16.to_le_bytes());
        assert_eq!(
            decode_artifact(&previous_version).unwrap_err().to_string(),
            "unsupported artifact version 2"
        );

        let mut unknown_version = bytes.clone();
        unknown_version[ARTIFACT_MAGIC.len()..ARTIFACT_MAGIC.len() + 2]
            .copy_from_slice(&(ARTIFACT_VERSION + 1).to_le_bytes());
        assert!(
            decode_artifact(&unknown_version)
                .unwrap_err()
                .to_string()
                .contains("version")
        );

        let mut wrong_length = bytes;
        let start = ARTIFACT_MAGIC.len() + 2;
        wrong_length[start..start + 8].copy_from_slice(&0u64.to_le_bytes());
        assert!(
            decode_artifact(&wrong_length)
                .unwrap_err()
                .to_string()
                .contains("length")
        );
    }

    #[test]
    fn artifact_rejects_trailing_bytes_and_unknown_hosts() {
        let script = compile("test.velin", "perform say(1)\n").unwrap();
        let mut bytes = encode_artifact("test.velin", &script).unwrap();
        bytes.push(0);
        assert!(decode_artifact(&bytes).is_err());

        let mut invalid = script.clone();
        invalid.hosts.clear();
        let error = encode_artifact("test.velin", &invalid).unwrap_err();
        assert!(error.to_string().contains("unknown host"));
    }

    #[test]
    fn cache_key_changes_with_semantic_inputs_and_cache_round_trips() {
        let script = compile("test.velin", "set x = 1\n").unwrap();
        let first = artifact_cache_key(b"set x = 1\n", "compiler-a", "speed", b"schema-a");
        let second = artifact_cache_key(b"set x = 1\n", "compiler-b", "speed", b"schema-a");
        assert_ne!(first, second);
        let directory =
            std::env::temp_dir().join(format!("velin-artifact-cache-{}", std::process::id()));
        let path = artifact_cache_path(&directory, &first);
        store_artifact_cache(&path, "test.velin", &script).unwrap();
        let cached = load_artifact_cache(&path).unwrap().unwrap();
        assert_eq!(cached.script().program, script.program);
        let _ = std::fs::remove_dir_all(directory);
    }
}
