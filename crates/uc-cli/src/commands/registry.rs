//! Native Scarb registry client (`scarbs.xyz`).
//!
//! Resolves `[dependencies]` / `[dev-dependencies]` registry packages (honouring `Scarb.lock`),
//! downloads the `.tar.zst` archives, verifies their SHA-256 checksum against the index, and
//! extracts them into the same cache layout scarb uses
//! (`<scarb cache>/registry/src/<registry-id>/<name>-<version>/`). Both tools then share one
//! package cache and `uc` no longer needs a scarb binary to build registry-dependency projects.

use super::*;
use sha2::{Digest, Sha256};
use std::io::Read;

const DEFAULT_REGISTRY_API: &str = "https://scarbs.xyz/api/v1";
/// Directory name scarb derives from the registry URL; kept literal so the two caches coincide.
const DEFAULT_REGISTRY_SRC_DIR: &str = "scarbs.xyz-9djtpev4jug5q";
/// Packages that ship inside the compiler / `uc` itself and never come from the registry.
const BUILTIN_PACKAGES: &[&str] = &[
    "starknet",
    "core",
    "assert_macros",
    "cairo_execute",
    "cairo_test",
    "cairo_run",
    "snforge_std",
    "sncast_std",
    // Prebuilt procedural-macro plugins are not Cairo sources; the compiler never needs them.
    "snforge_scarb_plugin",
];

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct RegistryPackageSpec {
    pub name: String,
    pub version: String,
    pub checksum: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub(crate) struct RegistryFetchReport {
    pub schema_version: u32,
    pub registry: String,
    pub cache_dir: String,
    /// Packages that were already present in the shared cache.
    pub cached: Vec<RegistryPackageSpec>,
    /// Packages downloaded, verified and extracted by this call.
    pub fetched: Vec<RegistryPackageSpec>,
    /// `Scarb.lock` written by this call (only when the package had none).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lockfile_written: Option<String>,
    pub elapsed_ms: f64,
}

#[derive(Debug, Deserialize)]
struct IndexDependency {
    name: String,
    req: String,
    #[serde(default)]
    kind: Option<String>,
}

#[derive(Debug, Deserialize)]
struct IndexEntry {
    v: String,
    #[serde(default)]
    deps: Vec<IndexDependency>,
    cksum: String,
}

/// Returns true when native registry fetching is enabled (`UC_REGISTRY_FETCH=0` disables it).
pub(crate) fn registry_fetch_enabled() -> bool {
    !matches!(
        std::env::var("UC_REGISTRY_FETCH").ok().as_deref().map(str::trim),
        Some("0" | "false" | "off" | "no")
    )
}

fn registry_api_base() -> String {
    std::env::var("UC_REGISTRY_API")
        .ok()
        .map(|v| v.trim().trim_end_matches('/').to_string())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| DEFAULT_REGISTRY_API.to_string())
}

/// Scarb's cache root on this platform (`SCARB_CACHE` wins), creating it if missing.
pub(crate) fn scarb_cache_root() -> Result<PathBuf> {
    if let Ok(custom) = std::env::var("SCARB_CACHE") {
        let p = PathBuf::from(custom.trim());
        if !custom.trim().is_empty() {
            return Ok(p);
        }
    }
    let home = std::env::var("HOME").context("HOME is not set; cannot locate the scarb cache")?;
    let home = PathBuf::from(home);
    if cfg!(target_os = "macos") {
        return Ok(home.join("Library/Caches/com.swmansion.scarb"));
    }
    if let Ok(xdg) = std::env::var("XDG_CACHE_HOME") {
        if !xdg.trim().is_empty() {
            return Ok(PathBuf::from(xdg.trim()).join("scarb"));
        }
    }
    Ok(home.join(".cache/scarb"))
}

/// `<cache>/registry/src/<registry-id>`; reuses whatever `scarbs.xyz-*` dir scarb already made.
pub(crate) fn registry_src_dir() -> Result<PathBuf> {
    let src_root = scarb_cache_root()?.join("registry/src");
    if let Ok(entries) = fs::read_dir(&src_root) {
        let mut found: Vec<PathBuf> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                p.is_dir()
                    && p.file_name()
                        .and_then(|n| n.to_str())
                        .is_some_and(|n| n.starts_with("scarbs.xyz-"))
            })
            .collect();
        found.sort();
        if let Some(first) = found.into_iter().next() {
            return Ok(first);
        }
    }
    Ok(src_root.join(DEFAULT_REGISTRY_SRC_DIR))
}

/// Package names are Cairo identifiers (letters, digits, `_`; `-` is tolerated): anything else,
/// such as `/` or `..`, would reach a URL or a cache path.
pub(crate) fn validate_package_name(name: &str) -> Result<()> {
    let ok = !name.is_empty()
        && name.len() <= 64
        && !name.starts_with('-')
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
    if !ok {
        bail!("refusing registry package name `{name}`: package names are letters, digits and `_`");
    }
    Ok(())
}

/// Sparse-index path prefix, identical to the cargo/scarb layout.
pub(crate) fn index_prefix(name: &str) -> String {
    let lower = name.to_ascii_lowercase();
    match lower.len() {
        0 => "0".to_string(),
        1 => "1".to_string(),
        2 => "2".to_string(),
        3 => format!("3/{}", &lower[..1]),
        _ => format!("{}/{}", &lower[..2], &lower[2..4]),
    }
}

fn fetch_index(name: &str) -> Result<Vec<IndexEntry>> {
    let url = format!("{}/index/{}/{}.json", registry_api_base(), index_prefix(name), name);
    let body = http_get_bytes(&url).with_context(|| format!("package `{name}` not found in registry index"))?;
    serde_json::from_slice::<Vec<IndexEntry>>(&body)
        .with_context(|| format!("registry index for `{name}` is not valid JSON"))
}

/// Highest non-prerelease version in the index satisfying every requirement in `reqs`
/// (bare versions are caret requirements).
fn pick_version<'a>(entries: &'a [IndexEntry], reqs: &[String]) -> Option<&'a IndexEntry> {
    let parsed: Vec<semver::VersionReq> = reqs
        .iter()
        .map(|r| semver::VersionReq::parse(r.trim()))
        .collect::<Result<_, _>>()
        .ok()?;
    entries
        .iter()
        .filter_map(|e| semver::Version::parse(&e.v).ok().map(|v| (v, e)))
        .filter(|(v, _)| v.pre.is_empty() && parsed.iter().all(|r| r.matches(v)))
        .max_by(|a, b| a.0.cmp(&b.0))
        .map(|(_, e)| e)
}

/// A registry package pinned by `uc.lock` or `Scarb.lock`.
struct LockedPackage {
    version: String,
    checksum: Option<String>,
    dependencies: Vec<String>,
}

/// Git dependency specification extracted from a manifest (`[dependencies]`, `[dev-dependencies]`,
/// or `[workspace.dependencies]`).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct GitDependencySpec {
    pub name: String,
    pub git: String,
    pub tag: Option<String>,
    pub branch: Option<String>,
    pub rev: Option<String>,
}

/// Reads `uc.lock` (preferred) or `Scarb.lock` from `manifest_dir` (or ancestor workspace root).
fn read_lockfile_toml(manifest_dir: &Path) -> Option<TomlValue> {
    for ancestor in manifest_dir.ancestors().take(4) {
        for lock_name in ["uc.lock", "Scarb.lock"] {
            let candidate = ancestor.join(lock_name);
            if let Ok(text) = fs::read_to_string(&candidate) {
                if let Ok(lock) = toml::from_str::<TomlValue>(&text) {
                    return Some(lock);
                }
            }
        }
    }
    None
}

/// Registry packages pinned by `uc.lock` or `Scarb.lock`, keyed by name.
fn locked_registry_packages(manifest_dir: &Path) -> BTreeMap<String, LockedPackage> {
    let mut out = BTreeMap::new();
    let Some(lock) = read_lockfile_toml(manifest_dir) else {
        return out;
    };
    for pkg in lock.get("package").and_then(TomlValue::as_array).into_iter().flatten() {
        let (Some(name), Some(version)) = (
            pkg.get("name").and_then(TomlValue::as_str),
            pkg.get("version").and_then(TomlValue::as_str),
        ) else {
            continue;
        };
        let source = pkg.get("source").and_then(TomlValue::as_str).unwrap_or("");
        if !source.starts_with("registry+") {
            continue;
        }
        let checksum = pkg.get("checksum").and_then(TomlValue::as_str).map(str::to_string);
        let dependencies = pkg
            .get("dependencies")
            .and_then(TomlValue::as_array)
            .into_iter()
            .flatten()
            .filter_map(TomlValue::as_str)
            .map(str::to_string)
            .collect();
        out.insert(name.to_string(), LockedPackage { version: version.to_string(), checksum, dependencies });
    }
    out
}

/// Git commit revisions pinned by `uc.lock` or `Scarb.lock` (`source = "git+...#<rev>"`), keyed by package name.
fn locked_git_revisions(manifest_dir: &Path) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    let Some(lock) = read_lockfile_toml(manifest_dir) else {
        return out;
    };
    for pkg in lock.get("package").and_then(TomlValue::as_array).into_iter().flatten() {
        let Some(name) = pkg.get("name").and_then(TomlValue::as_str) else {
            continue;
        };
        let Some(source) = pkg.get("source").and_then(TomlValue::as_str) else {
            continue;
        };
        if let Some((_, rev)) = source.strip_prefix("git+").and_then(|s| s.rsplit_once('#')) {
            let rev = rev.trim();
            if !rev.is_empty() {
                out.insert(name.to_string(), rev.to_string());
            }
        }
    }
    out
}

/// Finds the workspace manifest (`[workspace]`) in `manifest` or an ancestor directory.
fn find_workspace_manifest(manifest_dir: &Path, manifest: &TomlValue) -> Option<TomlValue> {
    if manifest.get("workspace").is_some() {
        return Some(manifest.clone());
    }
    for ancestor in manifest_dir.ancestors().skip(1).take(4) {
        for file_name in ["uc.toml", "Scarb.toml"] {
            let candidate = ancestor.join(file_name);
            if candidate.is_file() {
                if let Ok(text) = fs::read_to_string(&candidate) {
                    if let Ok(parsed) = toml::from_str::<TomlValue>(&text) {
                        if parsed.get("workspace").is_some() {
                            return Some(parsed);
                        }
                    }
                }
            }
        }
    }
    None
}

/// Resolves `{ workspace = true }` against `[workspace.dependencies]` if present.
fn resolve_workspace_dep_value<'a>(
    name: &str,
    value: &'a TomlValue,
    manifest: &'a TomlValue,
    workspace_manifest: Option<&'a TomlValue>,
) -> &'a TomlValue {
    let is_ws = value
        .as_table()
        .and_then(|t| t.get("workspace"))
        .and_then(TomlValue::as_bool)
        .unwrap_or(false);
    if !is_ws {
        return value;
    }
    for candidate in [Some(manifest), workspace_manifest].into_iter().flatten() {
        if let Some(entry) = candidate
            .get("workspace")
            .and_then(TomlValue::as_table)
            .and_then(|w| w.get("dependencies"))
            .and_then(TomlValue::as_table)
            .and_then(|d| d.get(name))
        {
            return entry;
        }
    }
    value
}

/// Direct registry dependencies declared by a manifest: `(name, version requirement)`.
fn manifest_registry_requirements_with_workspace(
    manifest: &TomlValue,
    workspace_manifest: Option<&TomlValue>,
) -> Vec<(String, String)> {
    manifest_registry_requirements_inner(manifest, workspace_manifest, true)
}

fn manifest_registry_requirements_inner(
    manifest: &TomlValue,
    workspace_manifest: Option<&TomlValue>,
    include_dev_dependencies: bool,
) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut tables: Vec<&toml::map::Map<String, TomlValue>> = Vec::new();
    let sections: &[&str] = if include_dev_dependencies {
        &["dependencies", "dev-dependencies", "dev_dependencies"]
    } else {
        &["dependencies"]
    };
    for section in sections {
        if let Some(table) = manifest.get(*section).and_then(TomlValue::as_table) {
            tables.push(table);
        }
    }
    if manifest.get("package").is_none() && manifest.get("project").is_none() {
        if let Some(ws_deps) = manifest
            .get("workspace")
            .and_then(TomlValue::as_table)
            .and_then(|w| w.get("dependencies"))
            .and_then(TomlValue::as_table)
        {
            tables.push(ws_deps);
        }
    }
    for table in tables {
        for (name, raw_value) in table {
            if BUILTIN_PACKAGES.contains(&name.as_str()) {
                continue;
            }
            let value = resolve_workspace_dep_value(name, raw_value, manifest, workspace_manifest);
            let req = match value {
                TomlValue::String(s) => s.clone(),
                TomlValue::Table(t) => {
                    if t.contains_key("path") || t.contains_key("git") {
                        continue;
                    }
                    if t.get("workspace").and_then(TomlValue::as_bool) == Some(true)
                        && t.get("version").is_none()
                    {
                        // Unresolved workspace dependency (likely local path in parent workspace).
                        continue;
                    }
                    match t.get("version").and_then(TomlValue::as_str) {
                        Some(v) => v.to_string(),
                        None => "*".to_string(),
                    }
                }
                _ => continue,
            };
            if !out.iter().any(|(n, r)| n == name && r == &req) {
                out.push((name.clone(), req));
            }
        }
    }
    out
}

/// Direct git dependencies declared by a manifest (`git = "https://..."`).
fn manifest_git_requirements_with_workspace(
    manifest: &TomlValue,
    workspace_manifest: Option<&TomlValue>,
    locked_revs: &BTreeMap<String, String>,
) -> Vec<GitDependencySpec> {
    manifest_git_requirements_inner(manifest, workspace_manifest, locked_revs, true)
}

fn manifest_git_requirements_inner(
    manifest: &TomlValue,
    workspace_manifest: Option<&TomlValue>,
    locked_revs: &BTreeMap<String, String>,
    include_dev_dependencies: bool,
) -> Vec<GitDependencySpec> {
    let mut out = Vec::new();
    let mut tables: Vec<&toml::map::Map<String, TomlValue>> = Vec::new();
    let sections: &[&str] = if include_dev_dependencies {
        &["dependencies", "dev-dependencies", "dev_dependencies"]
    } else {
        &["dependencies"]
    };
    for section in sections {
        if let Some(table) = manifest.get(*section).and_then(TomlValue::as_table) {
            tables.push(table);
        }
    }
    if let Some(ws_deps) = manifest
        .get("workspace")
        .and_then(TomlValue::as_table)
        .and_then(|w| w.get("dependencies"))
        .and_then(TomlValue::as_table)
    {
        tables.push(ws_deps);
    }
    for table in tables {
        for (name, raw_value) in table {
            if BUILTIN_PACKAGES.contains(&name.as_str()) {
                continue;
            }
            let value = resolve_workspace_dep_value(name, raw_value, manifest, workspace_manifest);
            let Some(t) = value.as_table() else {
                continue;
            };
            let Some(git_url) = t.get("git").and_then(TomlValue::as_str).map(str::trim) else {
                continue;
            };
            if git_url.is_empty() {
                continue;
            }
            let tag = t.get("tag").and_then(TomlValue::as_str).map(|s| s.trim().to_string());
            let branch = t.get("branch").and_then(TomlValue::as_str).map(|s| s.trim().to_string());
            let rev = t
                .get("rev")
                .and_then(TomlValue::as_str)
                .map(|s| s.trim().to_string())
                .or_else(|| locked_revs.get(name).cloned());
            let spec = GitDependencySpec {
                name: name.clone(),
                git: git_url.to_string(),
                tag,
                branch,
                rev,
            };
            if !out.contains(&spec) {
                out.push(spec);
            }
        }
    }
    out
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex::encode(hasher.finalize())
}

/// Minimal ustar/pax reader: extracts regular files of a `.tar` stream into `dest`, dropping
/// the archive's single top-level directory (`<name>-<version>/`).
pub(crate) fn extract_tar_strip_root(tar: &[u8], dest: &Path) -> Result<usize> {
    let mut offset = 0usize;
    let mut written = 0usize;
    let mut pax_path: Option<String> = None;
    while offset + 512 <= tar.len() {
        let header = &tar[offset..offset + 512];
        if header.iter().all(|b| *b == 0) {
            break;
        }
        let size_field = std::str::from_utf8(&header[124..136]).unwrap_or("0");
        let size = usize::from_str_radix(size_field.trim_matches(|c| c == '\0' || c == ' '), 8)
            .unwrap_or(0);
        let typeflag = header[156];
        let name_raw = String::from_utf8_lossy(&header[0..100]).trim_end_matches('\0').to_string();
        let prefix_raw = String::from_utf8_lossy(&header[345..500]).trim_end_matches('\0').to_string();
        let data_start = offset + 512;
        let data_end = (data_start + size).min(tar.len());
        let data = &tar[data_start..data_end];
        let padded = (size + 511) / 512 * 512;
        offset = data_start + padded;

        match typeflag {
            b'x' => {
                // PAX extended header: may carry a long `path=`.
                for record in String::from_utf8_lossy(data).split('\n') {
                    if let Some((_, rest)) = record.split_once(' ') {
                        if let Some(p) = rest.strip_prefix("path=") {
                            pax_path = Some(p.to_string());
                        }
                    }
                }
                continue;
            }
            b'L' => {
                // GNU LongLink extended filename for the next entry.
                let long_name = String::from_utf8_lossy(data)
                    .trim_end_matches('\0')
                    .trim_end_matches('\n')
                    .to_string();
                if !long_name.is_empty() {
                    pax_path = Some(long_name);
                }
                continue;
            }
            b'g' | b'K' => continue,
            b'0' | 0 => {}
            _ => {
                // Consume any PAX/LongLink path attached to a non-file entry (such as a directory)
                // so it never leaks onto the next regular file entry.
                let _ = pax_path.take();
                continue;
            }
        }
        let full_name = pax_path.take().unwrap_or_else(|| {
            if prefix_raw.is_empty() {
                name_raw.clone()
            } else {
                format!("{prefix_raw}/{name_raw}")
            }
        });
        let Some((_, rel)) = full_name.split_once('/') else {
            continue;
        };
        if rel.is_empty()
            || rel.split('/').any(|c| c == ".." || c.is_empty())
            || rel.starts_with('/')
            || rel.contains('\\')
            || rel.contains(':')
        {
            bail!("refusing to extract unsafe archive path `{full_name}`");
        }
        let target = dest.join(rel);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&target, data)?;
        written += 1;
    }
    Ok(written)
}

/// Largest package a registry archive may expand to; a bigger one is refused, not unpacked.
const MAX_PACKAGE_BYTES: u64 = 256 * 1024 * 1024;

fn decompress_zstd(bytes: &[u8]) -> Result<Vec<u8>> {
    let decoder = ruzstd::decoding::StreamingDecoder::new(bytes)
        .map_err(|e| anyhow!("invalid zstd archive: {e}"))?;
    let mut out = Vec::with_capacity(bytes.len().saturating_mul(8).min(MAX_PACKAGE_BYTES as usize));
    decoder
        .take(MAX_PACKAGE_BYTES + 1)
        .read_to_end(&mut out)
        .map_err(|e| anyhow!("failed to decompress zstd archive: {e}"))?;
    if out.len() as u64 > MAX_PACKAGE_BYTES {
        bail!("registry archive expands past {MAX_PACKAGE_BYTES} bytes; refusing it");
    }
    Ok(out)
}

fn package_dir_ready(dir: &Path) -> bool {
    (dir.join("Scarb.toml").is_file() || dir.join("uc.toml").is_file())
        && dir.join("src/lib.cairo").is_file()
}

/// Sanitizes a git URL or ref string into a safe single directory component.
fn sanitize_git_slug(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len().min(48));
    for c in raw.chars() {
        if c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.' {
            out.push(c);
        } else if !out.ends_with('-') {
            out.push('-');
        }
        if out.len() >= 48 {
            break;
        }
    }
    let trimmed = out.trim_matches(|c| c == '-' || c == '.');
    if trimmed.is_empty() {
        "repo".to_string()
    } else {
        trimmed.to_string()
    }
}

fn repo_slug_from_git_url(git_url: &str) -> String {
    let trimmed = git_url
        .trim()
        .trim_end_matches('/')
        .trim_end_matches(".git");
    let tail = trimmed
        .rsplit(['/', ':'])
        .next()
        .unwrap_or("repo");
    let hash8 = &sha256_hex(git_url.trim().as_bytes())[..12];
    format!("{}-{hash8}", sanitize_git_slug(tail))
}

fn ref_slug_for_git_spec(spec: &GitDependencySpec) -> String {
    if let Some(rev) = spec.rev.as_deref().filter(|s| !s.is_empty()) {
        return sanitize_git_slug(&rev[..rev.len().min(16)]);
    }
    if let Some(tag) = spec.tag.as_deref().filter(|s| !s.is_empty()) {
        return format!("tag-{}", sanitize_git_slug(tag));
    }
    if let Some(branch) = spec.branch.as_deref().filter(|s| !s.is_empty()) {
        return format!("branch-{}", sanitize_git_slug(branch));
    }
    "HEAD".to_string()
}

/// Recursively collects candidate package directories (up to depth 3) under a git checkout root.
fn collect_checkout_package_dirs(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
    if depth > 3 {
        return;
    }
    if package_dir_ready(dir) {
        out.push(dir.to_path_buf());
    }
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if name.starts_with('.') || name == "target" || name == "tests" {
            continue;
        }
        if name == "src"
            && dir.join("src/lib.cairo").is_file()
            && !path.join("Scarb.toml").is_file()
            && !path.join("uc.toml").is_file()
        {
            continue;
        }
        collect_checkout_package_dirs(&path, depth + 1, out);
    }
}

/// Checks if a package `pkg_name` is present inside a single git checkout directory `rev_dir`,
/// returning `(package_dir, version, manifest_toml)` when found.
fn find_package_in_checkout_dir(
    rev_dir: &Path,
    pkg_name: &str,
) -> Option<(PathBuf, String, TomlValue)> {
    let ws_version = ["uc.toml", "Scarb.toml"]
        .into_iter()
        .find_map(|f| fs::read_to_string(rev_dir.join(f)).ok())
        .and_then(|t| toml::from_str::<TomlValue>(&t).ok())
        .and_then(|v| {
            v.get("workspace")
                .and_then(|w| w.get("package").or_else(|| w.get("project")))
                .and_then(|p| p.get("version"))
                .and_then(TomlValue::as_str)
                .map(str::to_string)
        });
    let mut candidate_dirs = Vec::new();
    collect_checkout_package_dirs(rev_dir, 0, &mut candidate_dirs);
    for pkg_dir in candidate_dirs {
        for manifest_name in ["uc.toml", "Scarb.toml"] {
            let manifest_file = pkg_dir.join(manifest_name);
            if !manifest_file.is_file() {
                continue;
            }
            let Ok(text) = fs::read_to_string(&manifest_file) else {
                continue;
            };
            let Ok(val) = toml::from_str::<TomlValue>(&text) else {
                continue;
            };
            let tbl = val
                .get("package")
                .or_else(|| val.get("project"))
                .and_then(TomlValue::as_table);
            let Some(tbl) = tbl else {
                continue;
            };
            if tbl.get("name").and_then(TomlValue::as_str) == Some(pkg_name) {
                let ver = tbl
                    .get("version")
                    .and_then(TomlValue::as_str)
                    .or(ws_version.as_deref())
                    .unwrap_or("0.1.0")
                    .to_string();
                return Some((pkg_dir, ver, val));
            }
        }
    }
    None
}

/// Searches all existing `<scarb_cache>/registry/git/checkouts/<repo>/<rev>` directories for `spec`.
fn find_cached_git_dependency(
    checkouts_root: &Path,
    spec: &GitDependencySpec,
) -> Option<(PathBuf, String, TomlValue)> {
    // 1. Check the exact deterministic checkout directory uc creates for this spec.
    let exact_dir = checkouts_root
        .join(repo_slug_from_git_url(&spec.git))
        .join(ref_slug_for_git_spec(spec));
    if exact_dir.is_dir() {
        if let Some(found) = find_package_in_checkout_dir(&exact_dir, &spec.name) {
            return Some(found);
        }
    }
    // 2. Also check any checkout directory created by Scarb (`<repo>-<hash>/<rev>`).
    let Ok(repo_entries) = fs::read_dir(checkouts_root) else {
        return None;
    };
    let expected_rev_prefix = spec.rev.as_deref().filter(|s| !s.is_empty());
    let mut fallback_match: Option<(PathBuf, String, TomlValue)> = None;
    for repo_entry in repo_entries.flatten() {
        let Ok(rev_entries) = fs::read_dir(repo_entry.path()) else {
            continue;
        };
        for rev_entry in rev_entries.flatten() {
            let rev_path = rev_entry.path();
            if !rev_path.is_dir() {
                continue;
            }
            let Some(found) = find_package_in_checkout_dir(&rev_path, &spec.name) else {
                continue;
            };
            if let Some(rev_prefix) = expected_rev_prefix {
                let rev_name = rev_path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                if rev_name.starts_with(rev_prefix) || rev_prefix.starts_with(rev_name) {
                    return Some(found);
                }
            } else if spec.tag.is_none() && spec.branch.is_none() {
                return Some(found);
            }
            if fallback_match.is_none() {
                fallback_match = Some(found);
            }
        }
    }
    // Only use a fallback Scarb checkout if no specific uncached tag/branch was requested,
    // or if the version matches the requested tag.
    if let Some(ref found) = fallback_match {
        if let Some(tag) = spec.tag.as_deref() {
            if found.1.trim_start_matches('v') == tag.trim_start_matches('v') {
                return fallback_match;
            }
        } else {
            return fallback_match;
        }
    }
    None
}

fn validate_git_arg_safe(arg: &str, kind: &str) -> Result<()> {
    let trimmed = arg.trim();
    if trimmed.is_empty() || trimmed.starts_with('-') || trimmed.contains('\0') {
        bail!("refusing unsafe git {kind} `{arg}`");
    }
    Ok(())
}

fn run_git_cmd(args: &[&str], cwd: Option<&Path>) -> Result<()> {
    let mut cmd = Command::new("git");
    cmd.args(args)
        .env("GIT_TERMINAL_PROMPT", "0");
    if let Some(dir) = cwd {
        cmd.current_dir(dir);
    }
    let output = cmd.output().context("failed to spawn `git`")?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("git {:?} failed: {}", args, stderr.trim());
    }
    Ok(())
}

/// Clones a git repository at the requested `tag`/`branch`/`rev` into `dest_dir`.
fn clone_git_dependency_checkout(spec: &GitDependencySpec, dest_dir: &Path) -> Result<()> {
    validate_git_arg_safe(&spec.git, "URL")?;
    if let Some(ref t) = spec.tag {
        validate_git_arg_safe(t, "tag")?;
    }
    if let Some(ref b) = spec.branch {
        validate_git_arg_safe(b, "branch")?;
    }
    if let Some(ref r) = spec.rev {
        validate_git_arg_safe(r, "rev")?;
    }
    let parent = dest_dir.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)?;
    let staging = parent.join(format!(
        ".uc-git-staging-{}-{}",
        ref_slug_for_git_spec(spec),
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&staging);
    let staging_str = staging.to_string_lossy().to_string();

    let clone_res: Result<()> = (|| {
        if let Some(rev) = spec.rev.as_deref().filter(|s| !s.is_empty()) {
            fs::create_dir_all(&staging)?;
            let shallow_ok = run_git_cmd(&["init", "-q"], Some(&staging))
                .and_then(|_| run_git_cmd(&["remote", "add", "origin", &spec.git], Some(&staging)))
                .and_then(|_| run_git_cmd(&["fetch", "-q", "--depth", "1", "origin", rev], Some(&staging)))
                .and_then(|_| run_git_cmd(&["checkout", "-q", "FETCH_HEAD"], Some(&staging)));
            if shallow_ok.is_err() {
                let _ = fs::remove_dir_all(&staging);
                run_git_cmd(&["clone", "-q", "--no-checkout", &spec.git, &staging_str], None)?;
                run_git_cmd(&["checkout", "-q", rev], Some(&staging))?;
            }
        } else if let Some(ref_name) = spec
            .tag
            .as_deref()
            .or(spec.branch.as_deref())
            .filter(|s| !s.is_empty())
        {
            let branch_ok = run_git_cmd(
                &["clone", "-q", "--depth", "1", "--branch", ref_name, &spec.git, &staging_str],
                None,
            );
            if branch_ok.is_err() {
                let _ = fs::remove_dir_all(&staging);
                run_git_cmd(&["clone", "-q", &spec.git, &staging_str], None)?;
                run_git_cmd(&["checkout", "-q", ref_name], Some(&staging))?;
            }
        } else {
            run_git_cmd(&["clone", "-q", "--depth", "1", &spec.git, &staging_str], None)?;
        }
        Ok(())
    })();

    if let Err(err) = clone_res {
        let _ = fs::remove_dir_all(&staging);
        return Err(err).with_context(|| format!("failed to clone git dependency `{}` from {}", spec.name, spec.git));
    }

    // Remove `.git` metadata directory to keep cache small and fast to scan, and mark `.scarb-ok`.
    let _ = fs::remove_dir_all(staging.join(".git"));
    let _ = fs::write(staging.join(".scarb-ok"), b"");
    let _ = fs::remove_dir_all(dest_dir);
    fs::rename(&staging, dest_dir)
        .with_context(|| format!("failed to move git checkout into {}", dest_dir.display()))?;
    Ok(())
}

/// Downloads, verifies and extracts one package into the shared cache.
fn fetch_package(src_dir: &Path, spec: &RegistryPackageSpec) -> Result<()> {
    let url = format!("{}/dl/{}/{}", registry_api_base(), spec.name, spec.version);
    let archive = http_get_bytes(&url)
        .with_context(|| format!("failed to download {}@{}", spec.name, spec.version))?;
    if let Some(expected) = spec.checksum.as_deref() {
        let expected_hex = expected.strip_prefix("sha256:").unwrap_or(expected);
        let actual = sha256_hex(&archive);
        if !actual.eq_ignore_ascii_case(expected_hex) {
            bail!(
                "checksum mismatch for {}@{}: registry says sha256:{expected_hex}, downloaded sha256:{actual}",
                spec.name,
                spec.version
            );
        }
    }
    let tar = decompress_zstd(&archive)?;
    let final_dir = src_dir.join(format!("{}-{}", spec.name, spec.version));
    let staging = src_dir.join(format!(".uc-staging-{}-{}-{}", spec.name, spec.version, std::process::id()));
    let _ = fs::remove_dir_all(&staging);
    fs::create_dir_all(&staging)?;
    let files = extract_tar_strip_root(&tar, &staging)?;
    if !package_dir_ready(&staging) {
        let _ = fs::remove_dir_all(&staging);
        bail!(
            "archive for {}@{} did not contain Scarb.toml and src/lib.cairo ({files} files)",
            spec.name,
            spec.version
        );
    }
    // Scarb marks a fully extracted package with this sentinel; keep the caches interchangeable.
    fs::write(staging.join(".scarb-ok"), b"")?;
    let _ = fs::remove_dir_all(&final_dir);
    fs::rename(&staging, &final_dir)
        .with_context(|| format!("failed to move {} into place", final_dir.display()))?;
    Ok(())
}

/// Resolves every registry and git dependency reachable from `manifest_path` and makes sure its
/// sources are in the shared cache. Missing packages are downloaded/cloned; cached ones are reported.
pub(crate) fn ensure_registry_dependencies(manifest_path: &Path) -> Result<RegistryFetchReport> {
    let started = Instant::now();
    let manifest_text = fs::read_to_string(manifest_path)
        .with_context(|| format!("failed to read {}", manifest_path.display()))?;
    let manifest: TomlValue = toml::from_str(&manifest_text)
        .with_context(|| format!("failed to parse {}", manifest_path.display()))?;
    let manifest_dir = manifest_path.parent().unwrap_or_else(|| Path::new("."));
    let workspace_manifest = find_workspace_manifest(manifest_dir, &manifest);
    let locked = locked_registry_packages(manifest_dir);
    let locked_revs = locked_git_revisions(manifest_dir);
    let src_dir = registry_src_dir()?;
    fs::create_dir_all(&src_dir)?;
    let git_checkouts_root = scarb_cache_root()?.join("registry/git/checkouts");

    let mut report = RegistryFetchReport {
        schema_version: UC_AGENT_JSON_SCHEMA_VERSION,
        registry: registry_api_base(),
        cache_dir: src_dir.display().to_string(),
        ..Default::default()
    };
    let is_uc_toml = manifest_path.file_name().and_then(|s| s.to_str()) == Some("uc.toml");
    let lock_existed = manifest_dir.join("uc.lock").is_file()
        || manifest_dir.join("Scarb.lock").is_file();

    let mut direct_requirements =
        manifest_registry_requirements_with_workspace(&manifest, workspace_manifest.as_ref());
    let mut git_queue = manifest_git_requirements_with_workspace(
        &manifest,
        workspace_manifest.as_ref(),
        &locked_revs,
    );
    let mut visited_git_specs = BTreeSet::new();

    // 1. Resolve and clone any git dependencies (including transitive git/registry dependencies).
    while let Some(git_spec) = git_queue.pop() {
        if BUILTIN_PACKAGES.contains(&git_spec.name.as_str()) {
            continue;
        }
        validate_package_name(&git_spec.name)?;
        if !visited_git_specs.insert(git_spec.clone()) {
            continue;
        }
        let (pkg_dir, version, pkg_manifest) =
            if let Some(found) = find_cached_git_dependency(&git_checkouts_root, &git_spec) {
                report.cached.push(RegistryPackageSpec {
                    name: git_spec.name.clone(),
                    version: found.1.clone(),
                    checksum: Some(format!("git+{}", git_spec.git)),
                });
                found
            } else {
                fs::create_dir_all(&git_checkouts_root)?;
                let target_checkout_dir = git_checkouts_root
                    .join(repo_slug_from_git_url(&git_spec.git))
                    .join(ref_slug_for_git_spec(&git_spec));
                if !target_checkout_dir.is_dir() {
                    clone_git_dependency_checkout(&git_spec, &target_checkout_dir)?;
                }
                let found = find_package_in_checkout_dir(&target_checkout_dir, &git_spec.name)
                    .ok_or_else(|| {
                        anyhow!(
                            "git repository `{}` did not contain package `{}`",
                            git_spec.git,
                            git_spec.name
                        )
                    })?;
                report.fetched.push(RegistryPackageSpec {
                    name: git_spec.name.clone(),
                    version: found.1.clone(),
                    checksum: Some(format!("git+{}", git_spec.git)),
                });
                found
            };
        let _ = version;
        let pkg_ws_manifest = find_workspace_manifest(&pkg_dir, &pkg_manifest);
        for (trans_name, trans_req) in
            manifest_registry_requirements_inner(&pkg_manifest, pkg_ws_manifest.as_ref(), false)
        {
            if !direct_requirements
                .iter()
                .any(|(n, r)| n == &trans_name && r == &trans_req)
            {
                direct_requirements.push((trans_name, trans_req));
            }
        }
        for trans_git in manifest_git_requirements_inner(
            &pkg_manifest,
            pkg_ws_manifest.as_ref(),
            &locked_revs,
            false,
        ) {
            if !visited_git_specs.contains(&trans_git) {
                git_queue.push(trans_git);
            }
        }
    }

    let mut queue: Vec<(String, String)> = direct_requirements.clone();
    let mut resolved: BTreeSet<String> = BTreeSet::new();
    let mut index_cache: BTreeMap<String, Vec<IndexEntry>> = BTreeMap::new();
    // name -> every requirement seen so far; a package gets one version satisfying all of them
    // (greedy, highest first; re-resolved when a later requirement excludes the earlier pick).
    let mut requirements: BTreeMap<String, Vec<String>> = BTreeMap::new();
    // name -> (version, checksum, dependency names), for writing a lockfile.
    let mut resolved_entries: BTreeMap<String, (String, Option<String>, Vec<String>)> = BTreeMap::new();

    while let Some((name, req)) = queue.pop() {
        if BUILTIN_PACKAGES.contains(&name.as_str()) {
            continue;
        }
        // Names come from manifests and from the registry's own index (transitive
        // dependencies) and end up in URLs and cache paths.
        validate_package_name(&name)?;
        let reqs = requirements.entry(name.clone()).or_default();
        if !reqs.contains(&req) {
            reqs.push(req.clone());
        }
        if resolved.contains(&name) {
            let still_ok = locked.contains_key(&name)
                || resolved_entries.get(&name).is_some_and(|(v, _, _)| {
                    match (semver::VersionReq::parse(req.trim()), semver::Version::parse(v)) {
                        (Ok(r), Ok(ver)) => r.matches(&ver),
                        _ => true,
                    }
                });
            if still_ok {
                continue;
            }
            // The earlier pick no longer satisfies every requirement: resolve again.
            resolved.remove(&name);
            report.cached.retain(|p| p.name != name);
            report.fetched.retain(|p| p.name != name);
        }
        resolved.insert(name.clone());
        let (version, checksum, deps): (String, Option<String>, Vec<(String, String)>) =
            if let Some(locked_pkg) = locked.get(&name) {
                // Walk the lock graph (exact versions, no index lookups needed) instead of
                // the index; the lock already names every transitive dependency.
                let version = &locked_pkg.version;
                let deps: Vec<(String, String)> = locked_pkg
                    .dependencies
                    .iter()
                    .map(|d| (d.clone(), "*".to_string()))
                    .collect();
                let dir = src_dir.join(format!("{name}-{version}"));
                if package_dir_ready(&dir) {
                    report.cached.push(RegistryPackageSpec {
                        name: name.clone(),
                        version: version.clone(),
                        checksum: locked_pkg.checksum.clone(),
                    });
                    queue.extend(deps);
                    continue;
                }
                let checksum = match &locked_pkg.checksum {
                    Some(c) => Some(c.clone()),
                    None => {
                        let entries = fetch_index(&name)?;
                        entries.iter().find(|e| &e.v == version).map(|e| e.cksum.clone())
                    }
                };
                (version.clone(), checksum, deps)
            } else {
                let all_reqs = requirements.get(&name).cloned().unwrap_or_default();
                let exact_cached_candidate = if std::env::var_os("UC_REGISTRY_INDEX_URL").is_none() {
                    all_reqs
                        .iter()
                        .find_map(|r| {
                            let base = r.trim().trim_start_matches(['^', '~', '=', ' ', '>']);
                            let ver = semver::Version::parse(base).ok()?;
                            let satisfies_all = all_reqs.iter().all(|req_str| {
                                semver::VersionReq::parse(req_str.trim())
                                    .map(|vr| vr.matches(&ver))
                                    .unwrap_or(true)
                            });
                            if !satisfies_all {
                                return None;
                            }
                            let dir = src_dir.join(format!("{name}-{ver}"));
                            if package_dir_ready(&dir) {
                                Some((ver.to_string(), dir))
                            } else {
                                None
                            }
                        })
                } else {
                    None
                };
                if let Some((cached_ver, cached_dir)) = exact_cached_candidate {
                    let cached_manifest_path = if cached_dir.join("Scarb.toml").is_file() {
                        cached_dir.join("Scarb.toml")
                    } else {
                        cached_dir.join("uc.toml")
                    };
                    let deps = fs::read_to_string(&cached_manifest_path)
                        .ok()
                        .and_then(|raw| raw.parse::<TomlValue>().ok())
                        .map(|doc| manifest_registry_requirements_inner(&doc, None, false))
                        .unwrap_or_default();
                    let mut dep_names: Vec<String> = deps
                        .iter()
                        .map(|(n, _)| n.clone())
                        .filter(|n| !BUILTIN_PACKAGES.contains(&n.as_str()))
                        .collect();
                    dep_names.sort();
                    resolved_entries.insert(
                        name.clone(),
                        (cached_ver.clone(), None, dep_names),
                    );
                    (cached_ver, None, deps)
                } else {
                    if !index_cache.contains_key(&name) {
                        index_cache.insert(name.clone(), fetch_index(&name)?);
                    }
                    let entries = &index_cache[&name];
                    let Some(entry) = pick_version(entries, &all_reqs) else {
                        let available: Vec<&str> = entries.iter().map(|e| e.v.as_str()).collect();
                        bail!(
                            "no version of `{name}` satisfies {} in the registry (available: {})",
                            all_reqs.join(" and "),
                            available.join(", ")
                        );
                    };
                    let deps: Vec<(String, String)> = entry
                        .deps
                        .iter()
                        .filter(|d| d.kind.as_deref().unwrap_or("normal") == "normal")
                        .map(|d| (d.name.clone(), d.req.clone()))
                        .collect();
                    let mut dep_names: Vec<String> = deps
                        .iter()
                        .map(|(n, _)| n.clone())
                        .filter(|n| !BUILTIN_PACKAGES.contains(&n.as_str()))
                        .collect();
                    dep_names.sort();
                    resolved_entries.insert(
                        name.clone(),
                        (entry.v.clone(), Some(entry.cksum.clone()), dep_names),
                    );
                    (entry.v.clone(), Some(entry.cksum.clone()), deps)
                }
            };
        queue.extend(deps);
        let spec = RegistryPackageSpec { name: name.clone(), version: version.clone(), checksum };
        let dir = src_dir.join(format!("{name}-{version}"));
        if package_dir_ready(&dir) {
            report.cached.push(spec);
        } else {
            fetch_package(&src_dir, &spec)?;
            report.fetched.push(spec);
        }
    }
    let in_scarb_cache = scarb_cache_root()
        .is_ok_and(|cache_root| manifest_dir.starts_with(cache_root));
    if !lock_existed && !resolved_entries.is_empty() && !in_scarb_cache {
        if let Some(lock_path) = write_lockfile(
            manifest_dir,
            &manifest,
            &direct_requirements,
            &resolved_entries,
            is_uc_toml,
        )? {
            report.lockfile_written = Some(lock_path.display().to_string());
        }
    }
    report.elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;
    Ok(report)
}

/// Writes `uc.lock` (for `uc.toml`) or `Scarb.lock` (for `Scarb.toml`) (version 1) for a package
/// that had none: the root package with its registry dependencies, then every resolved registry
/// package with its source, checksum and dependency names, sorted by name.
fn write_lockfile(
    manifest_dir: &Path,
    manifest: &TomlValue,
    direct_requirements: &[(String, String)],
    resolved: &BTreeMap<String, (String, Option<String>, Vec<String>)>,
    is_uc_toml: bool,
) -> Result<Option<PathBuf>> {
    let Some(package) = manifest
        .get("package")
        .or_else(|| manifest.get("project"))
        .and_then(TomlValue::as_table)
    else {
        return Ok(None);
    };
    let Some(name) = package.get("name").and_then(TomlValue::as_str) else {
        return Ok(None);
    };
    let version = package
        .get("version")
        .and_then(TomlValue::as_str)
        .or_else(|| {
            manifest
                .get("workspace")
                .and_then(|w| w.get("package").or_else(|| w.get("project")))
                .and_then(|p| p.get("version"))
                .and_then(TomlValue::as_str)
        })
        .unwrap_or("0.1.0");
    let mut root_deps: Vec<String> = direct_requirements.iter().map(|(n, _)| n.clone()).collect();
    root_deps.sort();
    root_deps.dedup();
    let mut entries: Vec<(String, String, Option<String>, Vec<String>)> = resolved
        .iter()
        .map(|(n, (v, c, d))| (n.clone(), v.clone(), c.clone(), d.clone()))
        .collect();
    entries.push((name.to_string(), version.to_string(), None, root_deps));
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    let header = if is_uc_toml {
        "# Code generated by uc DO NOT EDIT.\nversion = 1\n"
    } else {
        "# Code generated by scarb DO NOT EDIT.\nversion = 1\n"
    };
    let mut out = String::from(header);
    for (n, v, checksum, deps) in entries {
        out.push_str(&format!("\n[[package]]\nname = \"{n}\"\nversion = \"{v}\"\n"));
        if checksum.is_some() {
            out.push_str("source = \"registry+https://scarbs.xyz/\"\n");
        }
        if let Some(c) = checksum {
            let c = if c.starts_with("sha256:") { c } else { format!("sha256:{c}") };
            out.push_str(&format!("checksum = \"{c}\"\n"));
        }
        if !deps.is_empty() {
            out.push_str("dependencies = [\n");
            for d in deps {
                out.push_str(&format!(" \"{d}\",\n"));
            }
            out.push_str("]\n");
        }
    }
    let lock_filename = if is_uc_toml { "uc.lock" } else { "Scarb.lock" };
    let lock_path = manifest_dir.join(lock_filename);
    fs::write(&lock_path, out).with_context(|| format!("failed to write {}", lock_path.display()))?;
    Ok(Some(lock_path))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn index_prefix_matches_sparse_layout() {
        assert_eq!(index_prefix("a"), "1");
        assert_eq!(index_prefix("ab"), "2");
        assert_eq!(index_prefix("abc"), "3/a");
        assert_eq!(index_prefix("openzeppelin_token"), "op/en");
        assert_eq!(index_prefix("Alexandria"), "al/ex");
    }

    #[test]
    fn pick_version_uses_caret_semantics_and_skips_prereleases() {
        let entries = vec![
            IndexEntry { v: "0.19.0".into(), deps: vec![], cksum: "a".into() },
            IndexEntry { v: "0.20.0".into(), deps: vec![], cksum: "b".into() },
            IndexEntry { v: "0.20.1-rc.1".into(), deps: vec![], cksum: "c".into() },
            IndexEntry { v: "1.0.0".into(), deps: vec![], cksum: "d".into() },
        ];
        let one = |r: &str| vec![r.to_string()];
        assert_eq!(pick_version(&entries, &one("0.20.0")).map(|e| e.v.as_str()), Some("0.20.0"));
        assert_eq!(pick_version(&entries, &one("^0.19")).map(|e| e.v.as_str()), Some("0.19.0"));
        assert_eq!(pick_version(&entries, &one(">=0.1")).map(|e| e.v.as_str()), Some("1.0.0"));
        assert_eq!(pick_version(&entries, &one("=0.19.0")).map(|e| e.v.as_str()), Some("0.19.0"));
        assert!(pick_version(&entries, &one("2.0.0")).is_none());
        // Two requirements on the same package unify to one version satisfying both.
        let both = vec![">=0.19".to_string(), "^0.20".to_string()];
        assert_eq!(pick_version(&entries, &both).map(|e| e.v.as_str()), Some("0.20.0"));
    }

    fn tar_entry(name: &str, data: &[u8]) -> Vec<u8> {
        let mut header = vec![0u8; 512];
        header[..name.len()].copy_from_slice(name.as_bytes());
        header[100..107].copy_from_slice(b"0000644");
        let size = format!("{:011o}", data.len());
        header[124..135].copy_from_slice(size.as_bytes());
        header[156] = b'0';
        header[257..262].copy_from_slice(b"ustar");
        let checksum: u32 = header.iter().map(|b| *b as u32).sum::<u32>() + 8 * 32;
        header[148..155].copy_from_slice(format!("{checksum:06o}\0").as_bytes());
        let mut out = header;
        out.extend_from_slice(data);
        out.resize(out.len() + (512 - data.len() % 512) % 512, 0);
        out
    }

    #[test]
    fn package_names_that_could_reach_a_path_are_refused() {
        for good in ["openzeppelin_token", "alexandria_math", "snforge_std", "garaga", "a-b"] {
            assert!(validate_package_name(good).is_ok(), "{good}");
        }
        for bad in ["", "../x", "a/b", "..", "-x", "a\\b", "a b", "a.b"] {
            assert!(validate_package_name(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn extract_tar_strips_the_top_level_package_dir_and_rejects_traversal() {
        let mut tar = tar_entry("pkg-1.0.0/Scarb.toml", b"[package]\nname = \"pkg\"\n");
        tar.extend(tar_entry("pkg-1.0.0/src/lib.cairo", b"fn main() {}\n"));
        tar.extend(vec![0u8; 1024]);
        let dest = std::env::temp_dir().join(format!("uc-registry-tar-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dest);
        let written = extract_tar_strip_root(&tar, &dest).unwrap();
        assert_eq!(written, 2);
        assert!(package_dir_ready(&dest));
        let evil = tar_entry("pkg-1.0.0/../escape.cairo", b"x");
        assert!(extract_tar_strip_root(&evil, &dest).is_err());
        let _ = fs::remove_dir_all(&dest);
    }

    #[test]
    fn clone_git_dependency_checkout_fetches_tagged_monorepo_package() {
        let base = std::env::temp_dir().join(format!("uc-git-dep-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        let remote_repo = base.join("remote-repo");
        let pkg_dir = remote_repo.join("packages/my_math");
        fs::create_dir_all(pkg_dir.join("src")).unwrap();
        fs::write(
            pkg_dir.join("Scarb.toml"),
            "[package]\nname = \"my_math\"\nversion = \"0.4.2\"\nedition = \"2024_07\"\n",
        )
        .unwrap();
        fs::write(
            pkg_dir.join("src/lib.cairo"),
            "pub fn double(x: u32) -> u32 { x * 2 }\n",
        )
        .unwrap();

        run_git_cmd(&["init", "-q"], Some(&remote_repo)).unwrap();
        run_git_cmd(&["config", "user.email", "test@uc.dev"], Some(&remote_repo)).unwrap();
        run_git_cmd(&["config", "user.name", "uc-test"], Some(&remote_repo)).unwrap();
        run_git_cmd(&["add", "."], Some(&remote_repo)).unwrap();
        run_git_cmd(&["commit", "-q", "-m", "initial"], Some(&remote_repo)).unwrap();
        run_git_cmd(&["tag", "v0.4.2"], Some(&remote_repo)).unwrap();

        let checkouts_root = base.join("cache/registry/git/checkouts");
        let spec = GitDependencySpec {
            name: "my_math".to_string(),
            git: remote_repo.to_string_lossy().to_string(),
            tag: Some("v0.4.2".to_string()),
            branch: None,
            rev: None,
        };
        assert!(find_cached_git_dependency(&checkouts_root, &spec).is_none());
        let target_dir = checkouts_root
            .join(repo_slug_from_git_url(&spec.git))
            .join(ref_slug_for_git_spec(&spec));
        clone_git_dependency_checkout(&spec, &target_dir).unwrap();
        let found = find_cached_git_dependency(&checkouts_root, &spec)
            .expect("cloned git dependency should be discovered in checkouts root");
        assert_eq!(found.1, "0.4.2");
        assert!(found.0.join("src/lib.cairo").is_file());
        let _ = fs::remove_dir_all(&base);
    }
}
