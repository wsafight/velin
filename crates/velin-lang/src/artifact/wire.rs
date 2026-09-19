//! Compact tagged wire encoding for artifact JSON payloads.

use super::{ArtifactError, MAX_ARTIFACT_BYTES, MAX_ARTIFACT_ENTRIES};
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use velin_bytecode::Program;

mod decode;
pub(super) use decode::decode_wire;

#[derive(Debug, Serialize, Deserialize)]
pub(super) struct TypeSiteWire {
    pub(super) pc: usize,
    pub(super) expression: velin_syntax::Expr,
    pub(super) kind: TypeCheckKindWire,
}

#[derive(Debug, Serialize, Deserialize)]
pub(super) enum TypeCheckKindWire {
    Expression,
    Condition,
    Assignment,
}

#[derive(Debug, Serialize, Deserialize)]
pub(super) struct HostSiteWire {
    pub(super) host_id: u32,
    pub(super) arguments: usize,
    pub(super) bind: bool,
    pub(super) line: usize,
}

pub(super) fn validate_check_sites(
    program: &Program,
    hosts: &[String],
    type_sites: &[TypeSiteWire],
    host_sites: &[HostSiteWire],
) -> Result<(), ArtifactError> {
    if type_sites.len() > MAX_ARTIFACT_ENTRIES || host_sites.len() > MAX_ARTIFACT_ENTRIES {
        return Err(ArtifactError::new(
            "artifact has too many static check sites",
        ));
    }
    if type_sites.iter().any(|site| site.pc >= program.ops.len()) {
        return Err(ArtifactError::new(
            "type check site points outside the program",
        ));
    }
    if host_sites
        .iter()
        .any(|site| usize::try_from(site.host_id).map_or(true, |id| id >= hosts.len()))
    {
        return Err(ArtifactError::new(
            "host check site references an unknown host id",
        ));
    }
    Ok(())
}

pub(super) fn encode_wire_value(
    value: &JsonValue,
    output: &mut Vec<u8>,
) -> Result<(), ArtifactError> {
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
