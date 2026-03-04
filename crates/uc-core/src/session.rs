use blake3::Hasher;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionInput {
    pub compiler_version: String,
    pub profile: String,
    pub offline: bool,
    pub package: Option<String>,
    pub features: Vec<String>,
    pub cfg_set: Vec<String>,
    pub manifest_content_hash: String,
    pub target_family: String,
    pub cairo_edition: Option<String>,
    pub cairo_lang_version: Option<String>,
    pub build_env_fingerprint: String,
}

#[derive(Debug, Serialize)]
struct NormalizedSessionInput<'a> {
    compiler_version: &'a str,
    profile: &'a str,
    offline: bool,
    package: Option<String>,
    features: Vec<String>,
    cfg_set: Vec<String>,
    manifest_content_hash: &'a str,
    target_family: &'a str,
    cairo_edition: Option<String>,
    cairo_lang_version: Option<String>,
    build_env_fingerprint: &'a str,
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
            offline: self.offline,
            package: self.package.clone(),
            features,
            cfg_set,
            manifest_content_hash: &self.manifest_content_hash,
            target_family: &self.target_family,
            cairo_edition: self.cairo_edition.clone(),
            cairo_lang_version: self.cairo_lang_version.clone(),
            build_env_fingerprint: &self.build_env_fingerprint,
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
            compiler_version: "2.14.0".to_string(),
            profile: "dev".to_string(),
            offline: false,
            package: Some("core".to_string()),
            features: features.into_iter().map(ToString::to_string).collect(),
            cfg_set: cfg_set.into_iter().map(ToString::to_string).collect(),
            manifest_content_hash: "manifest-blake3:abc".to_string(),
            target_family: "lib".to_string(),
            cairo_edition: Some("2024_07".to_string()),
            cairo_lang_version: Some("1.0".to_string()),
            build_env_fingerprint: "env-blake3:abc".to_string(),
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

    #[test]
    fn session_key_changes_when_offline_or_package_changes() {
        let a = sample_input(vec!["a"], vec!["cfg1"]);
        let mut b = a.clone();
        b.offline = true;
        assert_ne!(a.deterministic_key_hex(), b.deterministic_key_hex());

        let mut c = a.clone();
        c.package = Some("other".to_string());
        assert_ne!(a.deterministic_key_hex(), c.deterministic_key_hex());
    }

    #[test]
    fn session_key_changes_when_cairo_or_env_changes() {
        let a = sample_input(vec!["a"], vec!["cfg1"]);
        let mut b = a.clone();
        b.cairo_edition = Some("2023_11".to_string());
        assert_ne!(a.deterministic_key_hex(), b.deterministic_key_hex());

        let mut c = a.clone();
        c.build_env_fingerprint = "env-blake3:def".to_string();
        assert_ne!(a.deterministic_key_hex(), c.deterministic_key_hex());
    }
}
