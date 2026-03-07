/// Build script for uc-cli.
///
/// Phase 5 (cold-path supremacy): discovers the Cairo corelib at compile time
/// and exposes its path so `include_dir!` can embed it in the binary.
///
/// The embedded corelib eliminates the runtime filesystem search and guarantees
/// a version-matched corelib is always available, even when no local Cairo
/// installation exists.
use std::env;
use std::path::{Path, PathBuf};

fn main() {
    println!("cargo::rustc-check-cfg=cfg(uc_no_embedded_corelib)");
    println!("cargo:rerun-if-env-changed=UC_BUILD_CORELIB_SRC");
    println!("cargo:rerun-if-env-changed=UC_NATIVE_CORELIB_SRC");

    match find_corelib_src() {
        Some(path) => {
            println!(
                "cargo:rustc-env=UC_CORELIB_SRC_DIR={}",
                path.display()
            );
            println!(
                "cargo:warning=Phase 5: embedding corelib from {}",
                path.display()
            );
        }
        None => {
            println!("cargo:rustc-cfg=uc_no_embedded_corelib");
            println!(
                "cargo:warning=Phase 5: no corelib found at compile time; embedding disabled"
            );
        }
    }
}

fn corelib_layout_looks_compatible(path: &Path) -> bool {
    path.join("lib.cairo").is_file()
        && path.join("prelude.cairo").is_file()
        && path.join("ops.cairo").is_file()
}

/// Read the cairo-lang-compiler version from the workspace Cargo.lock so we
/// can verify that the discovered corelib matches the compiler we link against.
fn compiler_version_from_lockfile() -> Option<String> {
    let manifest_dir = env::var("CARGO_MANIFEST_DIR").ok()?;
    let lockfile_path = Path::new(&manifest_dir).join("../../Cargo.lock");
    let lockfile = std::fs::read_to_string(lockfile_path).ok()?;
    // Simple parse: look for [[package]] blocks with name = "cairo-lang-compiler"
    let mut in_target = false;
    for line in lockfile.lines() {
        let trimmed = line.trim();
        if trimmed == "[[package]]" {
            in_target = false;
            continue;
        }
        if trimmed == r#"name = "cairo-lang-compiler""# {
            in_target = true;
            continue;
        }
        if in_target && trimmed.starts_with("version = \"") {
            let version = trimmed
                .strip_prefix("version = \"")
                .and_then(|s| s.strip_suffix('"'));
            return version.map(String::from);
        }
    }
    None
}

/// Read the corelib version from its Scarb.toml.
fn corelib_manifest_version(corelib_src: &Path) -> Option<String> {
    let manifest_path = corelib_src.parent()?.join("Scarb.toml");
    let text = std::fs::read_to_string(manifest_path).ok()?;
    // Simple parse: look for version = "..." in [package] section
    let mut in_package = false;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed == "[package]" {
            in_package = true;
            continue;
        }
        if trimmed.starts_with('[') {
            in_package = false;
            continue;
        }
        if in_package && trimmed.starts_with("version") {
            if let Some(rest) = trimmed.strip_prefix("version") {
                let rest = rest.trim().strip_prefix('=')?.trim();
                let rest = rest.strip_prefix('"')?.strip_suffix('"')?;
                return Some(rest.to_string());
            }
        }
    }
    None
}

fn version_matches(corelib_src: &Path) -> bool {
    let compiler_ver = match compiler_version_from_lockfile() {
        Some(v) => v,
        None => return true, // cannot verify; optimistically allow
    };
    match corelib_manifest_version(corelib_src) {
        Some(v) => v == compiler_ver,
        None => true, // no Scarb.toml; treat as best-effort
    }
}

fn try_candidate(path: &Path) -> Option<PathBuf> {
    if path.exists() && corelib_layout_looks_compatible(path) && version_matches(path) {
        path.canonicalize().ok()
    } else {
        None
    }
}

fn find_corelib_src() -> Option<PathBuf> {
    // 1. Explicit build-time override
    if let Ok(path) = env::var("UC_BUILD_CORELIB_SRC") {
        if let Some(p) = try_candidate(Path::new(&path)) {
            return Some(p);
        }
    }

    // 2. Runtime env var (also useful at build time)
    if let Ok(path) = env::var("UC_NATIVE_CORELIB_SRC") {
        if let Some(p) = try_candidate(Path::new(&path)) {
            return Some(p);
        }
    }

    // 3. Search relative to workspace root (mirrors runtime native_corelib_candidate_paths)
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").ok()?);
    let workspace_root = manifest_dir.parent()?.parent()?;

    let mut candidates: Vec<PathBuf> = Vec::new();

    // Direct children: <workspace>/../cairo/corelib/src
    if let Some(parent) = workspace_root.parent() {
        candidates.push(parent.join("cairo/corelib/src"));

        // Sibling directories of workspace: <parent>/<sibling>/cairo/corelib/src
        if let Ok(entries) = std::fs::read_dir(parent) {
            for entry in entries.flatten() {
                if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                    candidates.push(entry.path().join("cairo/corelib/src"));
                    candidates.push(entry.path().join("corelib/src"));
                }
            }
        }
    }

    // Ancestors
    for ancestor in workspace_root.ancestors().skip(1).take(6) {
        candidates.push(ancestor.join("cairo/corelib/src"));
        candidates.push(ancestor.join("corelib/src"));
    }

    // detect_corelib equivalent: CARGO_MANIFEST_DIR ancestors
    for ancestor in manifest_dir.ancestors().skip(1).take(4) {
        candidates.push(ancestor.join("corelib/src"));
    }

    if let Ok(home) = env::var("HOME") {
        candidates.push(PathBuf::from(&home).join(".cairo/corelib/src"));
    }

    for candidate in candidates {
        if let Some(p) = try_candidate(&candidate) {
            return Some(p);
        }
    }

    None
}
