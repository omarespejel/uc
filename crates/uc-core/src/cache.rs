use blake3::Hasher;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ArtifactKeyInput {
    pub source_hash_hex: String,
    pub compiler_version: String,
    pub profile: String,
    pub features: Vec<String>,
    pub target_kind: String,
}

impl ArtifactKeyInput {
    pub fn digest_hex(&self) -> String {
        let mut features = self.features.clone();
        features.sort();

        let normalized = format!(
            "source_hash_hex={}\ncompiler_version={}\nprofile={}\nfeatures={}\ntarget_kind={}",
            self.source_hash_hex,
            self.compiler_version,
            self.profile,
            features.join(","),
            self.target_kind
        );

        let mut hasher = Hasher::new();
        hasher.update(normalized.as_bytes());
        hasher.finalize().to_hex().to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::ArtifactKeyInput;

    #[test]
    fn digest_is_stable_when_feature_order_changes() {
        let a = ArtifactKeyInput {
            source_hash_hex: "abc".to_string(),
            compiler_version: "2.14.0".to_string(),
            profile: "dev".to_string(),
            features: vec!["b".to_string(), "a".to_string()],
            target_kind: "lib".to_string(),
        };
        let b = ArtifactKeyInput {
            features: vec!["a".to_string(), "b".to_string()],
            ..a.clone()
        };

        assert_eq!(a.digest_hex(), b.digest_hex());
    }
}
