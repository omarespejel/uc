use anyhow::Result;
use blake3::Hasher;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
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

pub fn collect_artifact_digests(target_root: &Path) -> Result<Vec<ArtifactDigest>> {
    if !target_root.exists() {
        return Ok(Vec::new());
    }

    let mut digests = Vec::new();

    for entry in WalkDir::new(target_root)
        .follow_links(false)
        .into_iter()
        .filter_map(Result::ok)
    {
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

        let bytes = fs::read(path)?;
        let mut hasher = Hasher::new();
        hasher.update(&bytes);

        let relative = relative_path(target_root, path);
        digests.push(ArtifactDigest {
            relative_path: relative,
            blake3_hex: hasher.finalize().to_hex().to_string(),
            size_bytes: bytes.len() as u64,
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

#[cfg(test)]
mod tests {
    use super::{compare_artifact_sets, ArtifactDigest};

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
}
