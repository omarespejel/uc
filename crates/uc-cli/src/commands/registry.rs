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

/// A registry package pinned by `Scarb.lock`.
struct LockedPackage {
    version: String,
    checksum: Option<String>,
    dependencies: Vec<String>,
}

/// Registry packages pinned by `Scarb.lock`, keyed by name.
fn locked_registry_packages(manifest_dir: &Path) -> BTreeMap<String, LockedPackage> {
    let mut out = BTreeMap::new();
    let Ok(text) = fs::read_to_string(manifest_dir.join("Scarb.lock")) else {
        return out;
    };
    let Ok(lock) = toml::from_str::<TomlValue>(&text) else {
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

/// Direct registry dependencies declared by a manifest: `(name, version requirement)`.
fn manifest_registry_requirements(manifest: &TomlValue) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for section in ["dependencies", "dev-dependencies"] {
        let Some(table) = manifest.get(section).and_then(TomlValue::as_table) else {
            continue;
        };
        for (name, value) in table {
            if BUILTIN_PACKAGES.contains(&name.as_str()) {
                continue;
            }
            let req = match value {
                TomlValue::String(s) => s.clone(),
                TomlValue::Table(t) => {
                    if t.contains_key("path") || t.contains_key("git") {
                        continue;
                    }
                    match t.get("version").and_then(TomlValue::as_str) {
                        Some(v) => v.to_string(),
                        None => "*".to_string(),
                    }
                }
                _ => continue,
            };
            out.push((name.clone(), req));
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
            b'g' | b'L' | b'K' => continue,
            b'0' | 0 => {}
            _ => continue, // directories, symlinks, devices
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

fn decompress_zstd(bytes: &[u8]) -> Result<Vec<u8>> {
    let mut decoder = ruzstd::decoding::StreamingDecoder::new(bytes)
        .map_err(|e| anyhow!("invalid zstd archive: {e}"))?;
    let mut out = Vec::with_capacity(bytes.len() * 8);
    decoder
        .read_to_end(&mut out)
        .map_err(|e| anyhow!("failed to decompress zstd archive: {e}"))?;
    Ok(out)
}

fn package_dir_ready(dir: &Path) -> bool {
    dir.join("Scarb.toml").is_file() && dir.join("src/lib.cairo").is_file()
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

/// Resolves every registry dependency reachable from `manifest_path` and makes sure its sources
/// are in the shared cache. Missing packages are downloaded; cached ones are reported as such.
pub(crate) fn ensure_registry_dependencies(manifest_path: &Path) -> Result<RegistryFetchReport> {
    let started = Instant::now();
    let manifest_text = fs::read_to_string(manifest_path)
        .with_context(|| format!("failed to read {}", manifest_path.display()))?;
    let manifest: TomlValue = toml::from_str(&manifest_text)
        .with_context(|| format!("failed to parse {}", manifest_path.display()))?;
    let manifest_dir = manifest_path.parent().unwrap_or_else(|| Path::new("."));
    let locked = locked_registry_packages(manifest_dir);
    let src_dir = registry_src_dir()?;
    fs::create_dir_all(&src_dir)?;

    let mut report = RegistryFetchReport {
        schema_version: UC_AGENT_JSON_SCHEMA_VERSION,
        registry: registry_api_base(),
        cache_dir: src_dir.display().to_string(),
        ..Default::default()
    };
    let lock_existed = manifest_dir.join("Scarb.lock").is_file();
    let direct_requirements = manifest_registry_requirements(&manifest);
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
                if !index_cache.contains_key(&name) {
                    index_cache.insert(name.clone(), fetch_index(&name)?);
                }
                let entries = &index_cache[&name];
                let all_reqs = requirements.get(&name).cloned().unwrap_or_default();
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
    if !lock_existed && !resolved_entries.is_empty() {
        if let Some(lock_path) = write_lockfile(manifest_dir, &manifest, &direct_requirements, &resolved_entries)? {
            report.lockfile_written = Some(lock_path.display().to_string());
        }
    }
    report.elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;
    Ok(report)
}

/// Writes a scarb-compatible `Scarb.lock` (version 1) for a package that had none: the root
/// package with its registry dependencies, then every resolved registry package with its
/// source, checksum and dependency names, sorted by name like scarb does. Packages uc does not
/// resolve (`snforge_std`, `starknet`) are left for scarb to add on its next run.
fn write_lockfile(
    manifest_dir: &Path,
    manifest: &TomlValue,
    direct_requirements: &[(String, String)],
    resolved: &BTreeMap<String, (String, Option<String>, Vec<String>)>,
) -> Result<Option<PathBuf>> {
    let Some(package) = manifest.get("package").and_then(TomlValue::as_table) else {
        return Ok(None);
    };
    let (Some(name), Some(version)) = (
        package.get("name").and_then(TomlValue::as_str),
        package.get("version").and_then(TomlValue::as_str),
    ) else {
        return Ok(None);
    };
    let mut root_deps: Vec<String> = direct_requirements.iter().map(|(n, _)| n.clone()).collect();
    root_deps.sort();
    root_deps.dedup();
    let mut entries: Vec<(String, String, Option<String>, Vec<String>)> = resolved
        .iter()
        .map(|(n, (v, c, d))| (n.clone(), v.clone(), c.clone(), d.clone()))
        .collect();
    entries.push((name.to_string(), version.to_string(), None, root_deps));
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    let mut out = String::from("# Code generated by scarb DO NOT EDIT.\nversion = 1\n");
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
    let lock_path = manifest_dir.join("Scarb.lock");
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
}
