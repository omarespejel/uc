use anyhow::{bail, Context, Result};
use blake3::Hasher;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ArtifactDigest {
    pub relative_path: String,
    pub blake3_hex: String,
    pub size_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ArtifactMismatch {
    pub relative_path: String,
    pub baseline_hash: Option<String>,
    pub candidate_hash: Option<String>,
}

const DEFAULT_SUFFIXES: [&str; 5] = [
    ".sierra.json",
    ".sierra",
    ".casm",
    ".contract_class.json",
    ".executable.json",
];
const MAX_ARTIFACT_SIZE_BYTES: u64 = 64 * 1024 * 1024;
const SIERRA_ID_NORMALIZATION_SECTIONS: [&str; 2] = ["type_declarations", "libfunc_declarations"];

pub fn collect_artifact_digests(target_root: &Path) -> Result<Vec<ArtifactDigest>> {
    if !target_root.exists() {
        return Ok(Vec::new());
    }

    let mut digests = Vec::new();

    for entry in WalkDir::new(target_root).follow_links(false).into_iter() {
        let entry = entry.with_context(|| {
            format!(
                "failed to traverse artifact tree under {}",
                target_root.display()
            )
        })?;
        if !entry.file_type().is_file() {
            continue;
        }

        let path = entry.path();
        let Some(name) = path.file_name().and_then(|s| s.to_str()) else {
            continue;
        };

        if !DEFAULT_SUFFIXES.iter().any(|suffix| name.ends_with(suffix)) {
            continue;
        }

        let (blake3_hex, size_bytes) = if name.ends_with(".sierra.json") {
            hash_sierra_json_semantic(path)?
        } else {
            hash_file_with_limit(path)?
        };

        let relative = relative_path(target_root, path);
        digests.push(ArtifactDigest {
            relative_path: relative,
            blake3_hex,
            size_bytes,
        });
    }

    digests.sort_by(|a, b| a.relative_path.cmp(&b.relative_path));
    Ok(digests)
}

pub fn compare_artifact_sets(
    baseline: &[ArtifactDigest],
    candidate: &[ArtifactDigest],
) -> Vec<ArtifactMismatch> {
    let baseline_map = to_map(baseline);
    let candidate_map = to_map(candidate);

    let mut keys: Vec<String> = baseline_map
        .keys()
        .chain(candidate_map.keys())
        .cloned()
        .collect();
    keys.sort();
    keys.dedup();

    let mut mismatches = Vec::new();
    for key in keys {
        let left = baseline_map.get(&key);
        let right = candidate_map.get(&key);

        if left == right {
            continue;
        }

        mismatches.push(ArtifactMismatch {
            relative_path: key,
            baseline_hash: left.cloned(),
            candidate_hash: right.cloned(),
        });
    }

    mismatches
}

fn to_map(items: &[ArtifactDigest]) -> BTreeMap<String, String> {
    items
        .iter()
        .map(|item| (item.relative_path.clone(), item.blake3_hex.clone()))
        .collect()
}

fn relative_path(root: &Path, path: &Path) -> String {
    strip_prefix_safe(path, root)
        .unwrap_or_else(|| path.to_string_lossy().to_string())
        .replace('\\', "/")
}

fn strip_prefix_safe(path: &Path, root: &Path) -> Option<String> {
    let rel: PathBuf = path.strip_prefix(root).ok()?.to_path_buf();
    Some(rel.to_string_lossy().to_string())
}

fn hash_file_with_limit(path: &Path) -> Result<(String, u64)> {
    let metadata =
        fs::metadata(path).with_context(|| format!("failed to stat {}", path.display()))?;
    if metadata.len() > MAX_ARTIFACT_SIZE_BYTES {
        bail!(
            "artifact {} exceeds size limit ({} bytes > {} bytes)",
            path.display(),
            metadata.len(),
            MAX_ARTIFACT_SIZE_BYTES
        );
    }

    let file = File::open(path).with_context(|| format!("failed to open {}", path.display()))?;
    let mut reader = BufReader::new(file);
    let mut hasher = Hasher::new();
    let mut total = 0_u64;
    let mut buf = [0_u8; 8192];

    loop {
        let read = reader
            .read(&mut buf)
            .with_context(|| format!("failed to read {}", path.display()))?;
        if read == 0 {
            break;
        }
        total += read as u64;
        if total > MAX_ARTIFACT_SIZE_BYTES {
            bail!(
                "artifact {} exceeds size limit while streaming ({} bytes > {} bytes)",
                path.display(),
                total,
                MAX_ARTIFACT_SIZE_BYTES
            );
        }
        hasher.update(&buf[..read]);
    }

    Ok((hasher.finalize().to_hex().to_string(), total))
}

fn hash_sierra_json_semantic(path: &Path) -> Result<(String, u64)> {
    let metadata =
        fs::metadata(path).with_context(|| format!("failed to stat {}", path.display()))?;
    if metadata.len() > MAX_ARTIFACT_SIZE_BYTES {
        bail!(
            "artifact {} exceeds size limit ({} bytes > {} bytes)",
            path.display(),
            metadata.len(),
            MAX_ARTIFACT_SIZE_BYTES
        );
    }

    let bytes = fs::read(path).with_context(|| format!("failed to read {}", path.display()))?;
    let mut value: Value = serde_json::from_slice(&bytes)
        .with_context(|| format!("failed to parse JSON {}", path.display()))?;
    normalize_sierra_json_ids(&mut value);
    let canonical = canonicalize_json(&value);
    let canonical_bytes = serde_json::to_vec(&canonical)
        .with_context(|| format!("failed to serialize normalized JSON {}", path.display()))?;
    let mut hasher = Hasher::new();
    hasher.update(&canonical_bytes);
    Ok((hasher.finalize().to_hex().to_string(), metadata.len()))
}

fn normalize_sierra_json_ids(value: &mut Value) {
    normalize_sierra_json_ids_scoped(value, false);
}

fn normalize_sierra_json_ids_scoped(value: &mut Value, in_section: bool) {
    match value {
        Value::Object(map) => {
            for (key, item) in map.iter_mut() {
                let child_in_section = in_section
                    || SIERRA_ID_NORMALIZATION_SECTIONS
                        .iter()
                        .any(|section| *section == key);
                if key == "id" && in_section && item.is_number() {
                    *item = Value::String("<normalized-id>".to_string());
                } else {
                    normalize_sierra_json_ids_scoped(item, child_in_section);
                }
            }
        }
        Value::Array(items) => {
            for item in items {
                normalize_sierra_json_ids_scoped(item, in_section);
            }
        }
        _ => {}
    }
}

fn canonicalize_json(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut entries: Vec<_> = map.iter().collect();
            entries.sort_by(|(a, _), (b, _)| a.cmp(b));
            let mut canonical = serde_json::Map::new();
            for (key, item) in entries {
                canonical.insert(key.clone(), canonicalize_json(item));
            }
            Value::Object(canonical)
        }
        Value::Array(items) => Value::Array(items.iter().map(canonicalize_json).collect()),
        _ => value.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        canonicalize_json, compare_artifact_sets, normalize_sierra_json_ids, ArtifactDigest,
    };
    use serde_json::json;

    fn artifact(path: &str, hash: &str) -> ArtifactDigest {
        ArtifactDigest {
            relative_path: path.to_string(),
            blake3_hex: hash.to_string(),
            size_bytes: 1,
        }
    }

    #[test]
    fn compare_detects_missing_and_changed_files() {
        let baseline = vec![artifact("a.sierra.json", "111"), artifact("b.casm", "222")];
        let candidate = vec![
            artifact("a.sierra.json", "999"),
            artifact("c.sierra", "333"),
        ];

        let mismatches = compare_artifact_sets(&baseline, &candidate);
        assert_eq!(mismatches.len(), 3);
        assert!(mismatches
            .iter()
            .any(|m| m.relative_path == "a.sierra.json"
                && m.baseline_hash == Some("111".to_string())));
        assert!(mismatches
            .iter()
            .any(|m| m.relative_path == "b.casm" && m.candidate_hash.is_none()));
        assert!(mismatches
            .iter()
            .any(|m| m.relative_path == "c.sierra" && m.baseline_hash.is_none()));
    }

    #[test]
    fn sierra_normalization_ignores_numeric_ids() {
        let mut a = json!({
            "type_declarations": [{"id": 123, "debug_name": "felt252"}],
            "libfunc_declarations": [{"id": {"id": 7, "debug_name": "store_temp<felt252>"}}]
        });
        let mut b = json!({
            "type_declarations": [{"id": 999, "debug_name": "felt252"}],
            "libfunc_declarations": [{"id": {"id": 42, "debug_name": "store_temp<felt252>"}}]
        });

        normalize_sierra_json_ids(&mut a);
        normalize_sierra_json_ids(&mut b);

        assert_eq!(
            serde_json::to_string(&canonicalize_json(&a)).unwrap(),
            serde_json::to_string(&canonicalize_json(&b)).unwrap()
        );
    }

    #[test]
    fn sierra_normalization_preserves_unscoped_ids() {
        let mut value = json!({
            "metadata": {"id": 123, "name": "config"},
            "type_declarations": [{"id": 1, "debug_name": "felt252"}]
        });
        normalize_sierra_json_ids(&mut value);
        assert_eq!(value["metadata"]["id"], json!(123));
        assert_eq!(
            value["type_declarations"][0]["id"],
            json!("<normalized-id>")
        );
    }
}
