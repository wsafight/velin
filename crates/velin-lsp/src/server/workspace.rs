//! Bounded discovery of Velin sources under LSP workspace roots.

use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

const MAX_WORKSPACE_DOCUMENTS: usize = 128;
const MAX_WORKSPACE_BYTES: usize = 4 * 1024 * 1024;
const MAX_WORKSPACE_DEPTH: usize = 32;

pub(super) fn load_documents(initialize: &Value) -> HashMap<String, String> {
    let params = &initialize["params"];
    let mut roots = Vec::new();
    if let Some(folders) = params["workspaceFolders"].as_array() {
        roots.extend(
            folders
                .iter()
                .filter_map(|folder| folder["uri"].as_str())
                .filter_map(file_uri_to_path),
        );
    }
    if roots.is_empty() {
        roots.extend(params["rootUri"].as_str().and_then(file_uri_to_path));
    }
    if roots.is_empty() {
        roots.extend(params["rootPath"].as_str().map(PathBuf::from));
    }

    let mut paths = Vec::new();
    for root in roots {
        collect_sources(&root, 0, &mut paths);
        if paths.len() >= MAX_WORKSPACE_DOCUMENTS {
            break;
        }
    }
    paths.sort();
    paths.dedup();

    let mut documents = HashMap::new();
    let mut total_bytes = 0usize;
    for path in paths.into_iter().take(MAX_WORKSPACE_DOCUMENTS) {
        let Some(source) = read_source(&path) else {
            continue;
        };
        let Some(next_total) = total_bytes.checked_add(source.len()) else {
            break;
        };
        if next_total > MAX_WORKSPACE_BYTES {
            break;
        }
        total_bytes = next_total;
        documents.insert(path_to_file_uri(&path), source);
    }
    documents
}

pub(super) fn restore_closed_document(
    uri: &str,
    documents: &mut HashMap<String, String>,
    workspace_uris: &mut HashSet<String>,
) {
    if !workspace_uris.contains(uri) {
        documents.remove(uri);
        return;
    }
    let source = file_uri_to_path(uri).and_then(|path| read_source(&path));
    if let Some(source) = source {
        documents.insert(uri.to_owned(), source);
    } else {
        documents.remove(uri);
        workspace_uris.remove(uri);
    }
}

fn collect_sources(root: &Path, depth: usize, output: &mut Vec<PathBuf>) {
    if depth > MAX_WORKSPACE_DEPTH || output.len() >= MAX_WORKSPACE_DOCUMENTS {
        return;
    }
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    let mut entries: Vec<_> = entries.flatten().collect();
    entries.sort_by_key(std::fs::DirEntry::file_name);
    for entry in entries {
        if output.len() >= MAX_WORKSPACE_DOCUMENTS {
            return;
        }
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        let path = entry.path();
        if kind.is_dir() {
            if !is_ignored_directory(&path) {
                collect_sources(&path, depth + 1, output);
            }
        } else if kind.is_file()
            && path
                .extension()
                .is_some_and(|extension| extension == "velin")
        {
            output.push(path);
        }
    }
}

fn is_ignored_directory(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| matches!(name, ".git" | ".velin-cache" | "node_modules" | "target"))
}

fn read_source(path: &Path) -> Option<String> {
    let metadata = std::fs::metadata(path).ok()?;
    if metadata.len() > velin::MAX_SOURCE_BYTES as u64 {
        return None;
    }
    let bytes = std::fs::read(path).ok()?;
    (bytes.len() <= velin::MAX_SOURCE_BYTES)
        .then(|| String::from_utf8(bytes).ok())
        .flatten()
}

fn file_uri_to_path(uri: &str) -> Option<PathBuf> {
    let encoded = uri.strip_prefix("file://")?;
    if !encoded.starts_with('/') {
        return None;
    }
    let decoded = percent_decode(encoded)?;
    #[cfg(windows)]
    let decoded = decoded
        .strip_prefix('/')
        .filter(|path| path.as_bytes().get(1) == Some(&b':'))
        .unwrap_or(&decoded)
        .to_owned();
    Some(PathBuf::from(decoded))
}

fn path_to_file_uri(path: &Path) -> String {
    let raw = path.to_string_lossy().replace('\\', "/");
    let leading = if raw.starts_with('/') { "" } else { "/" };
    format!("file://{leading}{}", percent_encode(&raw))
}

fn percent_decode(value: &str) -> Option<String> {
    let bytes = value.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            let high = hex(*bytes.get(index + 1)?)?;
            let low = hex(*bytes.get(index + 2)?)?;
            decoded.push((high << 4) | low);
            index += 3;
        } else {
            decoded.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(decoded).ok()
}

fn percent_encode(value: &str) -> String {
    let mut encoded = String::new();
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~' | b'/' | b':') {
            encoded.push(char::from(byte));
        } else {
            use std::fmt::Write as _;
            let _ = write!(encoded, "%{byte:02X}");
        }
    }
    encoded
}

const fn hex(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}
