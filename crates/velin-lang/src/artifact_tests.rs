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
    assert_eq!(artifact.script().type_sites, script.type_sites);
    assert_eq!(artifact.script().host_sites.len(), script.host_sites.len());
}

#[test]
fn released_v4_artifact_fixture_remains_decodable() {
    let bytes = include_bytes!("../../../fixtures/compatibility/0.4.0/artifact-v4.velinc");
    let artifact = decode_artifact(bytes).unwrap();
    assert!(artifact.source_name().ends_with("source.velin"));
    assert_eq!(artifact.script().hosts, ["emit"]);
    assert_eq!(
        artifact.script().defaults.get("hp"),
        Some(&Value::Integer(3))
    );
    assert_eq!(artifact.script().program.ops.len(), 4);
}

#[test]
fn artifact_header_rejects_truncation_versions_and_length_mismatches() {
    let script = compile("test.velin", "set x = 1\n").unwrap();
    let bytes = encode_artifact("test.velin", &script).unwrap();
    assert!(decode_artifact(&bytes[..HEADER_BYTES - 1]).is_err());

    let mut previous_version = bytes.clone();
    previous_version[ARTIFACT_MAGIC.len()..ARTIFACT_MAGIC.len() + 2]
        .copy_from_slice(&(ARTIFACT_VERSION - 1).to_le_bytes());
    assert_eq!(
        decode_artifact(&previous_version).unwrap_err().to_string(),
        "unsupported artifact version 3"
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
fn artifact_rejects_duplicate_object_keys() {
    let script = compile("test.velin", "perform say(1)\n").unwrap();
    let mut bytes = encode_artifact("test.velin", &script).unwrap();
    let position = bytes
        .windows(b"type_sites".len())
        .position(|window| window == b"type_sites")
        .expect("artifact root contains type_sites");
    bytes[position..position + b"host_sites".len()].copy_from_slice(b"host_sites");

    let error = decode_artifact(&bytes).unwrap_err();
    assert!(error.to_string().contains("duplicate"), "{error}");
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
