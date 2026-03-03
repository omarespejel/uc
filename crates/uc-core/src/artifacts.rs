use anyhow::{bail, Context, Result};
use blake3::Hasher;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};
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
const SIERRA_NORMALIZATION_SCHEMA_TAG: &str = "sierra-normalization-v3";

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
    validate_supported_sierra_schema(&value, path)?;
    normalize_sierra_json_ids(&mut value);
    let canonical = canonicalize_json(&value);
    let canonical_bytes = serde_json::to_vec(&canonical)
        .with_context(|| format!("failed to serialize normalized JSON {}", path.display()))?;
    let mut hasher = Hasher::new();
    hasher.update(SIERRA_NORMALIZATION_SCHEMA_TAG.as_bytes());
    hasher.update(&canonical_bytes);
    Ok((hasher.finalize().to_hex().to_string(), metadata.len()))
}

fn validate_supported_sierra_schema(value: &Value, path: &Path) -> Result<()> {
    let Some(version_value) = value
        .get("sierra_format_version")
        .or_else(|| value.get("sierra_version"))
    else {
        return Ok(());
    };

    let version = match version_value {
        Value::String(text) => text.trim().to_string(),
        Value::Number(num) => num.to_string(),
        _ => bail!(
            "unsupported Sierra schema marker type in {} (expected string/number)",
            path.display()
        ),
    };

    let major = version.split('.').next().unwrap_or_default();
    if major != "1" {
        bail!(
            "unsupported Sierra schema version `{}` in {} (expected major version 1)",
            version,
            path.display()
        );
    }
    // Sierra v1 declaration IDs are compiler-internal numeric references. If this shape changes,
    // fail fast so we do not accidentally normalize semantically meaningful identifiers.
    validate_declaration_section_ids(value, "type_declarations", path)?;
    validate_declaration_section_ids(value, "libfunc_declarations", path)?;
    Ok(())
}

fn validate_declaration_section_ids(value: &Value, section: &str, path: &Path) -> Result<()> {
    let Some(items) = value.get(section).and_then(Value::as_array) else {
        return Ok(());
    };
    for (index, item) in items.iter().enumerate() {
        let id_value = item.get("id").with_context(|| {
            format!(
                "unsupported Sierra schema in {}: {}[{}] is missing `id`",
                path.display(),
                section,
                index
            )
        })?;
        if extract_numeric_id(Some(id_value)).is_none() {
            bail!(
                "unsupported Sierra schema in {}: {}[{}].id must be numeric",
                path.display(),
                section,
                index
            );
        }
    }
    Ok(())
}

fn normalize_sierra_json_ids(value: &mut Value) {
    let id_map = build_sierra_id_map(value);
    normalize_sierra_json_ids_with_map(value, &id_map);
}

fn build_sierra_id_map(value: &Value) -> HashMap<i64, String> {
    // For Sierra schema v1 (used by cairo-lang 2.14+), declaration IDs in these sections are
    // compiler-internal and safe to normalize without changing program semantics.
    let mut ids = HashMap::new();
    collect_section_ids(value, "type_declarations", "type", &mut ids);
    collect_section_ids(value, "libfunc_declarations", "libfunc", &mut ids);
    ids
}

fn collect_section_ids(value: &Value, section: &str, kind: &str, out: &mut HashMap<i64, String>) {
    let Some(items) = value.get(section).and_then(Value::as_array) else {
        return;
    };

    for (index, item) in items.iter().enumerate() {
        let Some(raw_id) = extract_numeric_id(item.get("id")) else {
            continue;
        };
        let debug_name = extract_debug_name(item)
            .unwrap_or_else(|| format!("{kind}-{index}"))
            .replace('\"', "");
        let token = format!("<{kind}:{index}:{debug_name}>");
        out.entry(raw_id).or_insert(token);
    }
}

fn extract_numeric_id(value: Option<&Value>) -> Option<i64> {
    match value? {
        Value::Number(num) => num
            .as_i64()
            .or_else(|| num.as_u64().and_then(|v| i64::try_from(v).ok())),
        Value::Object(map) => map
            .get("id")
            .and_then(|nested| extract_numeric_id(Some(nested))),
        _ => None,
    }
}

fn extract_debug_name(item: &Value) -> Option<String> {
    if let Some(name) = item
        .get("id")
        .and_then(Value::as_object)
        .and_then(|id| id.get("debug_name"))
        .and_then(Value::as_str)
    {
        return Some(name.to_string());
    }

    if let Some(name) = item
        .get("long_id")
        .and_then(Value::as_object)
        .and_then(|long| long.get("debug_name"))
        .and_then(Value::as_str)
    {
        return Some(name.to_string());
    }

    item.get("debug_name")
        .and_then(Value::as_str)
        .map(str::to_string)
}

#[derive(Copy, Clone)]
enum NormalizeContext {
    Default,
    FuncArray,
    FuncRoot,
}

fn normalize_sierra_json_ids_with_map(value: &mut Value, id_map: &HashMap<i64, String>) {
    normalize_sierra_json_ids_with_context(value, id_map, NormalizeContext::Default);
}

fn normalize_sierra_json_ids_with_context(
    value: &mut Value,
    id_map: &HashMap<i64, String>,
    context: NormalizeContext,
) {
    match value {
        Value::Object(map) => {
            for (key, item) in map.iter_mut() {
                if key == "id" && matches!(context, NormalizeContext::FuncRoot) {
                    continue;
                }
                if key == "id" && !matches!(context, NormalizeContext::FuncRoot) {
                    if let Some(raw_id) = extract_numeric_id(Some(item)) {
                        if let Some(mapped) = id_map.get(&raw_id) {
                            *item = Value::String(mapped.clone());
                            continue;
                        }
                    }
                }
                let next_context = if key == "funcs" {
                    NormalizeContext::FuncArray
                } else {
                    NormalizeContext::Default
                };
                normalize_sierra_json_ids_with_context(item, id_map, next_context);
            }
        }
        Value::Array(items) => {
            let item_context = if matches!(context, NormalizeContext::FuncArray) {
                NormalizeContext::FuncRoot
            } else {
                NormalizeContext::Default
            };
            for item in items {
                normalize_sierra_json_ids_with_context(item, id_map, item_context);
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
        canonicalize_json, compare_artifact_sets, normalize_sierra_json_ids,
        validate_supported_sierra_schema, ArtifactDigest,
    };
    use serde_json::json;
    use std::path::Path;

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
    fn sierra_normalization_preserves_function_ids_but_normalizes_type_ids() {
        let mut a = json!({
            "type_declarations": [{"id": {"id": 5, "debug_name": "felt252"}}],
            "funcs": [{
                "id": {"id": 10, "debug_name": "foo"},
                "signature": {"ret_types": [{"id": 5}]}
            }],
            "statements": [{
                "Invocation": {"libfunc_id": {"id": 7}}
            }]
        });
        let mut b = json!({
            "type_declarations": [{"id": {"id": 42, "debug_name": "felt252"}}],
            "funcs": [{
                "id": {"id": 99, "debug_name": "foo"},
                "signature": {"ret_types": [{"id": 42}]}
            }],
            "statements": [{
                "Invocation": {"libfunc_id": {"id": 7}}
            }]
        });

        normalize_sierra_json_ids(&mut a);
        normalize_sierra_json_ids(&mut b);

        assert_eq!(a["type_declarations"][0]["id"], json!("<type:0:felt252>"));
        assert_eq!(b["type_declarations"][0]["id"], json!("<type:0:felt252>"));
        assert_eq!(a["funcs"][0]["id"]["id"], json!(10));
        assert_eq!(b["funcs"][0]["id"]["id"], json!(99));
        assert_eq!(
            a["funcs"][0]["signature"]["ret_types"][0]["id"],
            json!("<type:0:felt252>")
        );
        assert_eq!(
            b["funcs"][0]["signature"]["ret_types"][0]["id"],
            json!("<type:0:felt252>")
        );
        assert_ne!(
            serde_json::to_string(&canonicalize_json(&a)).unwrap(),
            serde_json::to_string(&canonicalize_json(&b)).unwrap()
        );
    }

    #[test]
    fn sierra_normalization_preserves_unknown_ids() {
        let mut a = json!({
            "metadata": {"id": 12},
            "statements": [{"Invocation": {"branch": {"id": 7}}}]
        });
        let mut b = json!({
            "metadata": {"id": 12},
            "statements": [{"Invocation": {"branch": {"id": 9}}}]
        });
        normalize_sierra_json_ids(&mut a);
        normalize_sierra_json_ids(&mut b);
        assert_ne!(
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
            json!("<type:0:felt252>")
        );
    }

    #[test]
    fn sierra_schema_guard_accepts_major_version_one() {
        let value = json!({ "sierra_format_version": "1.5.0" });
        let result = validate_supported_sierra_schema(&value, Path::new("sample.sierra.json"));
        assert!(result.is_ok());
    }

    #[test]
    fn sierra_schema_guard_rejects_other_major_versions() {
        let value = json!({ "sierra_format_version": "2.0.0" });
        let err = validate_supported_sierra_schema(&value, Path::new("sample.sierra.json"))
            .expect_err("major version 2 should be rejected");
        assert!(format!("{err:#}").contains("unsupported Sierra schema version"));
    }

    #[test]
    fn sierra_schema_guard_rejects_non_numeric_declaration_ids() {
        let value = json!({
            "sierra_format_version": "1.0.0",
            "type_declarations": [{ "id": "felt252" }],
        });
        let err = validate_supported_sierra_schema(&value, Path::new("sample.sierra.json"))
            .expect_err("non-numeric declaration IDs should be rejected");
        assert!(format!("{err:#}").contains("unsupported Sierra schema"));
    }

    #[test]
    fn sierra_normalization_preserves_function_signature_semantics() {
        let mut a = json!({
            "type_declarations": [{"id": {"id": 5, "debug_name": "felt252"}}],
            "funcs": [{
                "id": {"id": 10, "debug_name": "foo"},
                "signature": {"ret_types": [{"id": 5}]}
            }]
        });
        let mut b = json!({
            "type_declarations": [{"id": {"id": 5, "debug_name": "u128"}}],
            "funcs": [{
                "id": {"id": 10, "debug_name": "foo"},
                "signature": {"ret_types": [{"id": 5}]}
            }]
        });

        normalize_sierra_json_ids(&mut a);
        normalize_sierra_json_ids(&mut b);

        assert_ne!(
            serde_json::to_string(&canonicalize_json(&a)).unwrap(),
            serde_json::to_string(&canonicalize_json(&b)).unwrap()
        );
    }
}
