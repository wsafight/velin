use super::*;
use crate::compile;

fn artifact_bytes() -> Vec<u8> {
    let script = compile("test.velin", "set x = 1\n").unwrap();
    encode_artifact("test.velin", &script).unwrap()
}

#[test]
fn decode_rejects_magic_unknown_tags_and_truncated_payloads() {
    let bytes = artifact_bytes();
    let mut bad_magic = bytes.clone();
    bad_magic[0] = b'X';
    assert!(
        decode_artifact(&bad_magic)
            .unwrap_err()
            .to_string()
            .contains("magic")
    );

    let mut unknown_tag = bytes.clone();
    unknown_tag[HEADER_BYTES] = 9;
    assert!(
        decode_artifact(&unknown_tag)
            .unwrap_err()
            .to_string()
            .contains("unknown wire tag")
    );

    let mut truncated = bytes.clone();
    truncated.pop();
    let start = ARTIFACT_MAGIC.len() + 2;
    let payload_len = u64::from_le_bytes(truncated[start..start + 8].try_into().unwrap());
    truncated[start..start + 8].copy_from_slice(&(payload_len - 1).to_le_bytes());
    assert!(decode_artifact(&truncated).is_err());
}

#[test]
fn cache_misses_and_malformed_entries_do_not_fail_the_caller() {
    let missing = std::env::temp_dir().join(format!("velin-missing-cache-{}", std::process::id()));
    assert!(load_artifact_cache(&missing).unwrap().is_none());

    let directory = std::env::temp_dir().join(format!("velin-cache-dir-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    assert!(load_artifact_cache(&directory).is_err());
    let _ = std::fs::remove_dir_all(&directory);

    let directory = std::env::temp_dir().join(format!("velin-bad-cache-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let path = directory.join("entry");
    std::fs::write(&path, b"not-an-artifact").unwrap();
    assert!(load_artifact_cache(&path).unwrap().is_none());
    let _ = std::fs::remove_dir_all(directory);

    let parent = std::env::temp_dir().join(format!("velin-cache-file-{}", std::process::id()));
    std::fs::write(&parent, b"file").unwrap();
    let script = compile("test.velin", "set x = 1\n").unwrap();
    assert!(store_artifact_cache(&parent.join("entry"), "test.velin", &script).is_err());
    let _ = std::fs::remove_file(parent);
}

#[test]
fn metadata_rejects_unknown_defaults_and_label_targets() {
    let mut script = compile("test.velin", "label start:\nset x = 1\n").unwrap();
    script.defaults.insert("missing".into(), Value::Integer(1));
    assert!(
        encode_artifact("test.velin", &script)
            .unwrap_err()
            .to_string()
            .contains("default")
    );

    let mut script = compile("test.velin", "label start:\nset x = 1\n").unwrap();
    script.labels.insert("start".into(), 99);
    assert!(
        encode_artifact("test.velin", &script)
            .unwrap_err()
            .to_string()
            .contains("outside")
    );
}

#[test]
fn artifact_error_from_program_validation_preserves_the_message() {
    let error = ArtifactError::from(ProgramValidationError {
        message: "bad program".into(),
    });
    assert!(error.to_string().contains("invalid artifact program"));
    assert!(error.to_string().contains("bad program"));
}
