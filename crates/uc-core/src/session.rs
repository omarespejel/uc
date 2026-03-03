use blake3::Hasher;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionInput {
    pub workspace_root: String,
    pub compiler_version: String,
    pub profile: String,
    pub features: Vec<String>,
    pub cfg_set: Vec<String>,
    pub plugin_signature: String,
    pub target_family: String,
}

impl SessionInput {
    pub fn deterministic_key_hex(&self) -> String {
        let mut features = self.features.clone();
        let mut cfg_set = self.cfg_set.clone();
        features.sort();
        cfg_set.sort();

        let normalized = format!(
            "workspace_root={}\ncompiler_version={}\nprofile={}\nfeatures={}\ncfg_set={}\nplugin_signature={}\ntarget_family={}",
            self.workspace_root,
            self.compiler_version,
            self.profile,
            features.join(","),
            cfg_set.join(","),
            self.plugin_signature,
            self.target_family
        );

        let mut hasher = Hasher::new();
        hasher.update(normalized.as_bytes());
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
            plugin_signature: "plugin-v1".to_string(),
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
