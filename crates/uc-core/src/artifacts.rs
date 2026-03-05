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

const DEFAULT_SUFFIXES: [&str; 7] = [
    ".sierra.json",
    ".sierra",
    ".casm",
    ".contract_class.json",
    ".compiled_contract_class.json",
    ".starknet_artifacts.json",
    ".executable.json",
];
const MAX_ARTIFACT_SIZE_BYTES: u64 = 64 * 1024 * 1024;
const SIERRA_NORMALIZATION_SCHEMA_TAG: &str = "sierra-normalization-v3";
const CONTRACT_CLASS_NORMALIZATION_SCHEMA_TAG: &str = "contract-class-normalization-v1";

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
        } else if name.ends_with(".compiled_contract_class.json") {
            hash_file_with_limit(path)?
        } else if name.ends_with(".contract_class.json") {
            hash_contract_class_json_semantic(path)?
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

fn hash_contract_class_json_semantic(path: &Path) -> Result<(String, u64)> {
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
    if contract_class_schema_marker(&value).is_none() {
        tracing::debug!(
            path = %path.display(),
            "contract class missing schema marker; falling back to raw hash"
        );
        return hash_file_with_limit(path);
    }
    if let Err(schema_err) = validate_supported_contract_class_schema(&value, path) {
        tracing::warn!(
            path = %path.display(),
            error = %format!("{schema_err:#}"),
            "unsupported contract-class schema; falling back to raw hash"
        );
        return hash_file_with_limit(path);
    }
    normalize_contract_class_compiler_version_triplet(&mut value);
    normalize_sierra_json_ids(&mut value);
    let canonical = canonicalize_json(&value);
    let canonical_bytes = serde_json::to_vec(&canonical)
        .with_context(|| format!("failed to serialize normalized JSON {}", path.display()))?;
    let mut hasher = Hasher::new();
    hasher.update(CONTRACT_CLASS_NORMALIZATION_SCHEMA_TAG.as_bytes());
    hasher.update(&canonical_bytes);
    Ok((hasher.finalize().to_hex().to_string(), metadata.len()))
}

fn contract_class_schema_marker(value: &Value) -> Option<&Value> {
    // `contract_class_version` appears in Starknet contract-class JSON.
    // `sierra_format_version` appears in raw Sierra program JSON.
    // `sierra_version` is treated as a legacy alias to keep normalization
    // forward-compatible with intermediate tooling outputs.
    value
        .get("contract_class_version")
        .or_else(|| value.get("sierra_format_version"))
        .or_else(|| value.get("sierra_version"))
}

fn contract_class_schema_version_text(
    version_value: &Value,
    field_name: &str,
    path: &Path,
) -> Result<String> {
    let version = match version_value {
        Value::String(text) => text.trim().to_string(),
        Value::Number(num) => num.to_string(),
        _ => bail!(
            "unsupported `{}` type in {} (expected string/number)",
            field_name,
            path.display()
        ),
    };
    if version.is_empty() {
        bail!("empty `{}` value in {}", field_name, path.display());
    }
    Ok(version)
}

fn validate_contract_class_schema_marker_major(
    version_value: &Value,
    field_name: &str,
    allowed_majors: &[&str],
    path: &Path,
) -> Result<()> {
    let version = contract_class_schema_version_text(version_value, field_name, path)?;
    let major = version.split('.').next().unwrap_or_default();
    if !allowed_majors.contains(&major) {
        let expected = allowed_majors.join(" or ");
        bail!(
            "unsupported `{}` version `{}` in {} (expected major version {})",
            field_name,
            version,
            path.display(),
            expected
        );
    }
    Ok(())
}

fn validate_supported_contract_class_schema(value: &Value, path: &Path) -> Result<()> {
    let Some(_) = contract_class_schema_marker(value) else {
        bail!("missing contract-class schema marker in {}", path.display());
    };
    if let Some(version_value) = value.get("contract_class_version") {
        validate_contract_class_schema_marker_major(
            version_value,
            "contract_class_version",
            &["0", "1"],
            path,
        )?;
    }
    if let Some(version_value) = value.get("sierra_format_version") {
        validate_contract_class_schema_marker_major(
            version_value,
            "sierra_format_version",
            &["1"],
            path,
        )?;
    }
    if let Some(version_value) = value.get("sierra_version") {
        validate_contract_class_schema_marker_major(version_value, "sierra_version", &["1"], path)?;
    }
    validate_declaration_section_ids(value, "type_declarations", path)?;
    validate_declaration_section_ids(value, "libfunc_declarations", path)?;
    validate_declaration_id_uniqueness(value, path)?;
    Ok(())
}

fn normalize_contract_class_compiler_version_triplet(value: &mut Value) {
    let Some(program) = value
        .get_mut("sierra_program")
        .and_then(Value::as_array_mut)
    else {
        return;
    };
    if program.len() < 6 {
        return;
    }
    let Some(sierra_major) = value_hex_u64(&program[0]) else {
        return;
    };
    if sierra_major != 1 {
        tracing::debug!(
            sierra_major,
            "normalize_contract_class_compiler_version_triplet: unrecognized Sierra major version; skipping normalization"
        );
        return;
    }
    if value_hex_u64(&program[1]).is_none()
        || value_hex_u64(&program[2]).is_none()
        || value_hex_u64(&program[3]).is_none()
        || value_hex_u64(&program[4]).is_none()
        || value_hex_u64(&program[5]).is_none()
    {
        return;
    }
    for index in 3..=5 {
        match program.get(index) {
            Some(Value::String(_)) => program[index] = Value::String("0x0".to_string()),
            Some(Value::Number(_)) => program[index] = Value::Number(serde_json::Number::from(0)),
            _ => return,
        }
    }
}

fn value_hex_u64(value: &Value) -> Option<u64> {
    match value {
        Value::String(text) => parse_hex_u64(text),
        Value::Number(number) => number.as_u64(),
        _ => None,
    }
}

fn parse_hex_u64(value: &str) -> Option<u64> {
    let trimmed = value.trim();
    if let Some(hex) = trimmed
        .strip_prefix("0x")
        .or_else(|| trimmed.strip_prefix("0X"))
    {
        return u64::from_str_radix(hex, 16).ok();
    }
    trimmed.parse::<u64>().ok()
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
    validate_declaration_id_uniqueness(value, path)?;
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
        validate_declaration_id_shape(id_value, path, section, index)?;
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

fn validate_declaration_id_shape(
    id_value: &Value,
    path: &Path,
    section: &str,
    index: usize,
) -> Result<()> {
    match id_value {
        Value::Number(_) => Ok(()),
        Value::Object(map) => {
            let nested_id = map.get("id").with_context(|| {
                format!(
                    "unsupported Sierra schema in {}: {}[{}].id object is missing nested `id`",
                    path.display(),
                    section,
                    index
                )
            })?;
            if extract_numeric_id(Some(nested_id)).is_none() {
                bail!(
                    "unsupported Sierra schema in {}: {}[{}].id nested value must be numeric",
                    path.display(),
                    section,
                    index
                );
            }
            if let Some(debug_name) = map.get("debug_name") {
                if !debug_name.is_string() {
                    bail!(
                        "unsupported Sierra schema in {}: {}[{}].id.debug_name must be a string",
                        path.display(),
                        section,
                        index
                    );
                }
            }
            for key in map.keys() {
                if key != "id" && key != "debug_name" {
                    bail!(
                        "unsupported Sierra schema in {}: {}[{}].id has unexpected key `{}`",
                        path.display(),
                        section,
                        index,
                        key
                    );
                }
            }
            Ok(())
        }
        _ => bail!(
            "unsupported Sierra schema in {}: {}[{}].id must be numeric or an object with numeric `id`",
            path.display(),
            section,
            index
        ),
    }
}

fn validate_declaration_id_uniqueness(value: &Value, path: &Path) -> Result<()> {
    let mut seen: HashMap<i64, (&'static str, usize)> = HashMap::new();
    for section in ["type_declarations", "libfunc_declarations"] {
        let Some(items) = value.get(section).and_then(Value::as_array) else {
            continue;
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
            let raw_id = extract_numeric_id(Some(id_value)).with_context(|| {
                format!(
                    "unsupported Sierra schema in {}: {}[{}].id must be numeric",
                    path.display(),
                    section,
                    index
                )
            })?;
            if let Some((prev_section, prev_index)) = seen.insert(raw_id, (section, index)) {
                bail!(
                    "unsupported Sierra schema in {}: declaration id `{}` is reused by {}[{}] and {}[{}]",
                    path.display(),
                    raw_id,
                    prev_section,
                    prev_index,
                    section,
                    index
                );
            }
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
        canonicalize_json, collect_artifact_digests, compare_artifact_sets,
        hash_contract_class_json_semantic, hash_file_with_limit, hash_sierra_json_semantic,
        normalize_sierra_json_ids, validate_supported_sierra_schema, ArtifactDigest,
    };
    use serde_json::json;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn artifact(path: &str, hash: &str) -> ArtifactDigest {
        ArtifactDigest {
            relative_path: path.to_string(),
            blake3_hex: hash.to_string(),
            size_bytes: 1,
        }
    }

    fn unique_test_path(name: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be after unix epoch")
            .as_nanos();
        std::env::temp_dir().join(format!(
            "uc-core-{name}-{}-{nonce}.json",
            std::process::id()
        ))
    }

    fn unique_test_dir(name: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be after unix epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("uc-core-{name}-{}-{nonce}", std::process::id()))
    }

    #[test]
    fn collect_artifact_digests_includes_native_compiled_and_manifest_suffixes() {
        let dir = unique_test_dir("artifact-suffixes");
        fs::create_dir_all(&dir).expect("failed to create artifact suffixes test directory");
        fs::write(
            dir.join("pkg_token.compiled_contract_class.json"),
            br#"{"test":"compiled"}"#,
        )
        .expect("failed to write compiled contract class artifact");
        fs::write(
            dir.join("pkg.starknet_artifacts.json"),
            br#"{"contracts":[]}"#,
        )
        .expect("failed to write starknet artifacts manifest");
        fs::write(
            dir.join("pkg_token.contract_class.json"),
            br#"{
                "contract_class_version":"0.1.0",
                "sierra_program":["0x1","0x7","0x0","0x2","0x10","0x0","0x2b9"],
                "type_declarations":[{"id":{"id":11,"debug_name":"felt252"}}],
                "libfunc_declarations":[{"id":{"id":41,"debug_name":"store_temp<felt252>"}}],
                "entry_points_by_type":{},
                "abi":""
            }"#,
        )
        .expect("failed to write contract class artifact");
        fs::write(dir.join("ignored.meta.json"), br#"{"ignored":true}"#)
            .expect("failed to write ignored artifact");

        let digests = collect_artifact_digests(&dir).expect("failed to collect artifact digests");
        let relative_paths: Vec<_> = digests
            .iter()
            .map(|item| item.relative_path.as_str())
            .collect();
        assert!(
            relative_paths.contains(&"pkg_token.compiled_contract_class.json"),
            "compiled contract class suffix should be included"
        );
        assert!(
            relative_paths.contains(&"pkg.starknet_artifacts.json"),
            "starknet artifacts manifest suffix should be included"
        );
        assert!(
            relative_paths.contains(&"pkg_token.contract_class.json"),
            "contract class suffix should be included"
        );
        assert_eq!(
            digests.len(),
            3,
            "unexpected digest set: {relative_paths:?}"
        );

        let _ = fs::remove_dir_all(&dir);
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
    fn sierra_schema_guard_rejects_unexpected_declaration_id_object_keys() {
        let value = json!({
            "sierra_format_version": "1.0.0",
            "type_declarations": [{
                "id": { "id": 7, "debug_name": "felt252", "semantic_id": "stable-name" }
            }],
        });
        let err = validate_supported_sierra_schema(&value, Path::new("sample.sierra.json"))
            .expect_err("unexpected declaration id object keys should be rejected");
        assert!(format!("{err:#}").contains("unexpected key"));
    }

    #[test]
    fn sierra_schema_guard_rejects_reused_declaration_ids() {
        let value = json!({
            "sierra_format_version": "1.0.0",
            "type_declarations": [{ "id": 11 }],
            "libfunc_declarations": [{ "id": 11 }],
        });
        let err = validate_supported_sierra_schema(&value, Path::new("sample.sierra.json"))
            .expect_err("reused declaration IDs should be rejected");
        assert!(format!("{err:#}").contains("is reused by"));
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

    #[test]
    fn sierra_semantic_hash_is_stable_across_declaration_id_renumbering() {
        let path_a = unique_test_path("sierra-hash-a");
        let path_b = unique_test_path("sierra-hash-b");
        let body_a = json!({
            "sierra_format_version": "1.0.0",
            "type_declarations": [{"id": {"id": 11, "debug_name": "felt252"}}],
            "libfunc_declarations": [{"id": {"id": 41, "debug_name": "store_temp<felt252>"}}],
            "funcs": [{
                "id": {"id": 9, "debug_name": "main"},
                "signature": {"ret_types": [{"id": 11}]}
            }],
        });
        let body_b = json!({
            "sierra_format_version": "1.0.0",
            "type_declarations": [{"id": {"id": 77, "debug_name": "felt252"}}],
            "libfunc_declarations": [{"id": {"id": 88, "debug_name": "store_temp<felt252>"}}],
            "funcs": [{
                "id": {"id": 9, "debug_name": "main"},
                "signature": {"ret_types": [{"id": 77}]}
            }],
        });
        fs::write(
            &path_a,
            serde_json::to_vec(&body_a).expect("failed to encode test sierra a"),
        )
        .expect("failed to write test sierra a");
        fs::write(
            &path_b,
            serde_json::to_vec(&body_b).expect("failed to encode test sierra b"),
        )
        .expect("failed to write test sierra b");

        let hash_a = hash_sierra_json_semantic(&path_a)
            .expect("failed to hash sierra a")
            .0;
        let hash_b = hash_sierra_json_semantic(&path_b)
            .expect("failed to hash sierra b")
            .0;
        assert_eq!(
            hash_a, hash_b,
            "semantic hash should be stable across declaration ID renumbering"
        );

        let _ = fs::remove_file(&path_a);
        let _ = fs::remove_file(&path_b);
    }

    #[test]
    fn sierra_semantic_hash_changes_when_signature_semantics_change() {
        let path_a = unique_test_path("sierra-semantic-a");
        let path_b = unique_test_path("sierra-semantic-b");
        let body_a = json!({
            "sierra_format_version": "1.0.0",
            "type_declarations": [{"id": {"id": 11, "debug_name": "felt252"}}],
            "libfunc_declarations": [{"id": {"id": 41, "debug_name": "store_temp<felt252>"}}],
            "funcs": [{
                "id": {"id": 9, "debug_name": "main"},
                "signature": {"ret_types": [{"id": 11}]}
            }],
        });
        let body_b = json!({
            "sierra_format_version": "1.0.0",
            "type_declarations": [{"id": {"id": 11, "debug_name": "u128"}}],
            "libfunc_declarations": [{"id": {"id": 41, "debug_name": "store_temp<u128>"}}],
            "funcs": [{
                "id": {"id": 9, "debug_name": "main"},
                "signature": {"ret_types": [{"id": 11}]}
            }],
        });
        fs::write(
            &path_a,
            serde_json::to_vec(&body_a).expect("failed to encode semantic test a"),
        )
        .expect("failed to write semantic test a");
        fs::write(
            &path_b,
            serde_json::to_vec(&body_b).expect("failed to encode semantic test b"),
        )
        .expect("failed to write semantic test b");

        let hash_a = hash_sierra_json_semantic(&path_a)
            .expect("failed to hash semantic a")
            .0;
        let hash_b = hash_sierra_json_semantic(&path_b)
            .expect("failed to hash semantic b")
            .0;
        assert_ne!(
            hash_a, hash_b,
            "semantic hash must change when declaration semantics change"
        );

        let _ = fs::remove_file(&path_a);
        let _ = fs::remove_file(&path_b);
    }

    #[test]
    fn contract_class_semantic_hash_ignores_compiler_version_triplet() {
        let path_a = unique_test_path("contract-hash-a");
        let path_b = unique_test_path("contract-hash-b");
        let body_a = json!({
            "contract_class_version": "0.1.0",
            "sierra_program": ["0x1", "0x7", "0x0", "0x2", "0xe", "0x0", "0x2b9"],
            "type_declarations": [{"id": {"id": 11, "debug_name": "felt252"}}],
            "libfunc_declarations": [{"id": {"id": 41, "debug_name": "store_temp<felt252>"}}],
        });
        let body_b = json!({
            "contract_class_version": "0.1.0",
            "sierra_program": ["0x1", "0x7", "0x0", "0x2", "0x10", "0x0", "0x2b9"],
            "type_declarations": [{"id": {"id": 11, "debug_name": "felt252"}}],
            "libfunc_declarations": [{"id": {"id": 41, "debug_name": "store_temp<felt252>"}}],
        });
        fs::write(
            &path_a,
            serde_json::to_vec(&body_a).expect("failed to encode contract hash a"),
        )
        .expect("failed to write contract hash a");
        fs::write(
            &path_b,
            serde_json::to_vec(&body_b).expect("failed to encode contract hash b"),
        )
        .expect("failed to write contract hash b");

        let hash_a = hash_contract_class_json_semantic(&path_a)
            .expect("failed to hash contract a")
            .0;
        let hash_b = hash_contract_class_json_semantic(&path_b)
            .expect("failed to hash contract b")
            .0;
        assert_eq!(
            hash_a, hash_b,
            "contract-class semantic hash should ignore compiler version triplet differences"
        );

        let _ = fs::remove_file(&path_a);
        let _ = fs::remove_file(&path_b);
    }

    #[test]
    fn contract_class_semantic_hash_changes_on_program_semantics() {
        let path_a = unique_test_path("contract-semantic-a");
        let path_b = unique_test_path("contract-semantic-b");
        let body_a = json!({
            "contract_class_version": "0.1.0",
            "sierra_program": ["0x1", "0x7", "0x0", "0x2", "0x10", "0x0", "0x2b9"],
            "type_declarations": [{"id": {"id": 11, "debug_name": "felt252"}}],
            "libfunc_declarations": [{"id": {"id": 41, "debug_name": "store_temp<felt252>"}}],
        });
        let body_b = json!({
            "contract_class_version": "0.1.0",
            "sierra_program": ["0x1", "0x7", "0x0", "0x2", "0x10", "0x0", "0x2ba"],
            "type_declarations": [{"id": {"id": 11, "debug_name": "felt252"}}],
            "libfunc_declarations": [{"id": {"id": 41, "debug_name": "store_temp<felt252>"}}],
        });
        fs::write(
            &path_a,
            serde_json::to_vec(&body_a).expect("failed to encode semantic contract a"),
        )
        .expect("failed to write semantic contract a");
        fs::write(
            &path_b,
            serde_json::to_vec(&body_b).expect("failed to encode semantic contract b"),
        )
        .expect("failed to write semantic contract b");

        let hash_a = hash_contract_class_json_semantic(&path_a)
            .expect("failed to hash semantic contract a")
            .0;
        let hash_b = hash_contract_class_json_semantic(&path_b)
            .expect("failed to hash semantic contract b")
            .0;
        assert_ne!(
            hash_a, hash_b,
            "contract-class semantic hash must change on Sierra program semantic changes"
        );

        let _ = fs::remove_file(&path_a);
        let _ = fs::remove_file(&path_b);
    }

    #[test]
    fn contract_class_semantic_hash_falls_back_to_raw_hash_for_unknown_schema_major() {
        let path = unique_test_path("contract-schema-fallback");
        let body = json!({
            "contract_class_version": "9.0.0",
            "sierra_program": ["0x1", "0x7", "0x0", "0x2", "0x10", "0x0", "0x2b9"],
            "type_declarations": [{"id": {"id": 11, "debug_name": "felt252"}}],
            "libfunc_declarations": [{"id": {"id": 41, "debug_name": "store_temp<felt252"}}],
        });
        fs::write(
            &path,
            serde_json::to_vec(&body).expect("failed to encode fallback contract"),
        )
        .expect("failed to write fallback contract");

        let semantic = hash_contract_class_json_semantic(&path)
            .expect("semantic hashing should fall back instead of failing");
        let raw = hash_file_with_limit(&path).expect("raw hash should succeed");
        assert_eq!(
            semantic, raw,
            "unsupported contract-class schema should fall back to raw hashing"
        );

        let _ = fs::remove_file(&path);
    }

    #[test]
    fn contract_class_semantic_hash_falls_back_when_sierra_format_major_is_unsupported() {
        let path = unique_test_path("contract-schema-sierra-format-major-fallback");
        let body = json!({
            "contract_class_version": "0.1.0",
            "sierra_format_version": "2.0.0",
            "sierra_program": ["0x1", "0x7", "0x0", "0x2", "0x10", "0x0", "0x2b9"],
            "type_declarations": [{"id": {"id": 11, "debug_name": "felt252"}}],
            "libfunc_declarations": [{"id": {"id": 41, "debug_name": "store_temp<felt252>"}}],
        });
        fs::write(
            &path,
            serde_json::to_vec(&body).expect("failed to encode fallback contract"),
        )
        .expect("failed to write fallback contract");

        let semantic = hash_contract_class_json_semantic(&path)
            .expect("semantic hashing should fall back instead of failing");
        let raw = hash_file_with_limit(&path).expect("raw hash should succeed");
        assert_eq!(
            semantic, raw,
            "unsupported sierra_format_version should fall back to raw hashing"
        );

        let _ = fs::remove_file(&path);
    }

    #[test]
    fn contract_class_semantic_hash_accepts_legacy_sierra_version_marker() {
        let path = unique_test_path("contract-schema-legacy-sierra-version");
        let body = json!({
            "sierra_version": "1.0.0",
            "sierra_program": ["0x1", "0x7", "0x0", "0x2", "0x10", "0x0", "0x2b9"],
            "type_declarations": [{"id": {"id": 11, "debug_name": "felt252"}}],
            "libfunc_declarations": [{"id": {"id": 41, "debug_name": "store_temp<felt252>"}}],
        });
        fs::write(
            &path,
            serde_json::to_vec(&body).expect("failed to encode contract"),
        )
        .expect("failed to write contract");

        let semantic = hash_contract_class_json_semantic(&path)
            .expect("legacy sierra_version marker should be accepted");
        let raw = hash_file_with_limit(&path).expect("raw hash should succeed");
        assert_ne!(
            semantic, raw,
            "recognized legacy schema marker should keep semantic hashing path"
        );

        let _ = fs::remove_file(&path);
    }

    #[test]
    fn contract_class_semantic_hash_falls_back_to_raw_hash_when_schema_marker_missing() {
        let path = unique_test_path("contract-schema-marker-missing");
        let body = json!({
            "sierra_program": ["0x1", "0x7", "0x0", "0x2", "0x10", "0x0", "0x2b9"],
            "type_declarations": [{"id": {"id": 11, "debug_name": "felt252"}}],
            "libfunc_declarations": [{"id": {"id": 41, "debug_name": "store_temp<felt252>"}}],
        });
        fs::write(
            &path,
            serde_json::to_vec(&body).expect("failed to encode marker-missing contract"),
        )
        .expect("failed to write marker-missing contract");

        let semantic = hash_contract_class_json_semantic(&path)
            .expect("semantic hashing should fall back when schema marker is missing");
        let raw = hash_file_with_limit(&path).expect("raw hash should succeed");
        assert_eq!(
            semantic, raw,
            "missing schema marker should fall back to raw hashing"
        );

        let _ = fs::remove_file(&path);
    }
}
