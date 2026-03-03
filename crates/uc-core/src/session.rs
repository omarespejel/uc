use blake3::Hasher;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionInput {
    pub workspace_root: String,
    pub compiler_version: String,
    pub profile: String,
    pub features: Vec<String>,
    pub cfg_set: Vec<String>,
    pub manifest_content_hash: String,
    pub target_family: String,
}

#[derive(Debug, Serialize)]
struct NormalizedSessionInput<'a> {
    compiler_version: &'a str,
    profile: &'a str,
    features: Vec<String>,
    cfg_set: Vec<String>,
    manifest_content_hash: &'a str,
    target_family: &'a str,
}

impl SessionInput {
    pub fn deterministic_key_hex(&self) -> String {
        let mut features = self.features.clone();
        let mut cfg_set = self.cfg_set.clone();
        features.sort_unstable();
        features.dedup();
        cfg_set.sort_unstable();
        cfg_set.dedup();

        let normalized = serde_json::to_vec(&NormalizedSessionInput {
            compiler_version: &self.compiler_version,
            profile: &self.profile,
            features,
            cfg_set,
            manifest_content_hash: &self.manifest_content_hash,
            target_family: &self.target_family,
        })
        .expect("session key normalization serialization must not fail");

        let mut hasher = Hasher::new();
        hasher.update(&normalized);
        hasher.finalize().to_hex().to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::SessionInput;

    fn sample_input(features: Vec<&str>, cfg_set: Vec<&str>) -> SessionInput {
        SessionInput {
            workspace_root: "/tmp/ws".to_string(),
            compiler_version: "2.14.0".to_string(),
            profile: "dev".to_string(),
            features: features.into_iter().map(ToString::to_string).collect(),
            cfg_set: cfg_set.into_iter().map(ToString::to_string).collect(),
            manifest_content_hash: "manifest-blake3:abc".to_string(),
            target_family: "lib".to_string(),
        }
    }

    #[test]
    fn session_key_is_order_independent_for_features_and_cfg() {
        let a = sample_input(vec!["b", "a"], vec!["cfg2", "cfg1"]);
        let b = sample_input(vec!["a", "b"], vec!["cfg1", "cfg2"]);
        assert_eq!(a.deterministic_key_hex(), b.deterministic_key_hex());
    }

    #[test]
    fn session_key_changes_when_profile_changes() {
        let mut a = sample_input(vec!["a"], vec!["cfg1"]);
        let mut b = a.clone();
        b.profile = "release".to_string();
        assert_ne!(a.deterministic_key_hex(), b.deterministic_key_hex());
        a.profile = "release".to_string();
        assert_eq!(a.deterministic_key_hex(), b.deterministic_key_hex());
    }
}
