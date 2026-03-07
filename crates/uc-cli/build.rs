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
    if let Some(lockfile) = workspace_lockfile_path() {
        println!("cargo:rerun-if-changed={}", lockfile.display());
    }
    let candidates = corelib_candidate_paths();
    emit_corelib_candidate_rerun_hints(&candidates);

    match find_corelib_src(&candidates) {
        Some(path) => {
            println!("cargo:rustc-env=UC_CORELIB_SRC_DIR={}", path.display());
            println!(
                "cargo:warning=Phase 5: embedding corelib from {}",
                path.display()
            );
        }
        None => {
            println!("cargo:rustc-cfg=uc_no_embedded_corelib");
            println!("cargo:warning=Phase 5: no corelib found at compile time; embedding disabled");
        }
    }
}

fn corelib_layout_looks_compatible(path: &Path) -> bool {
    path.join("lib.cairo").is_file()
        && path.join("prelude.cairo").is_file()
        && path.join("ops.cairo").is_file()
}

fn workspace_lockfile_path() -> Option<PathBuf> {
    let manifest_dir = env::var("CARGO_MANIFEST_DIR").ok()?;
    Some(Path::new(&manifest_dir).join("../../Cargo.lock"))
}

/// Read the cairo-lang-compiler version from the workspace Cargo.lock so we
/// can verify that the discovered corelib matches the compiler we link against.
fn compiler_version_from_lockfile() -> Option<String> {
    let lockfile_path = workspace_lockfile_path()?;
    let lockfile = std::fs::read_to_string(lockfile_path).ok()?;
    let parsed: toml::Value = toml::from_str(&lockfile).ok()?;
    let packages = parsed.get("package")?.as_array()?;
    packages.iter().find_map(|entry| {
        let table = entry.as_table()?;
        (table.get("name")?.as_str()? == "cairo-lang-compiler")
            .then(|| table.get("version")?.as_str().map(ToOwned::to_owned))
            .flatten()
    })
}

/// Read the corelib version from its Scarb.toml.
fn corelib_manifest_version(corelib_src: &Path) -> Option<String> {
    let manifest_path = corelib_src.parent()?.join("Scarb.toml");
    let text = std::fs::read_to_string(manifest_path).ok()?;
    let parsed: toml::Value = toml::from_str(&text).ok()?;
    parsed
        .get("package")
        .and_then(toml::Value::as_table)
        .and_then(|package| package.get("version"))
        .and_then(toml::Value::as_str)
        .map(ToOwned::to_owned)
}

fn version_matches(corelib_src: &Path) -> bool {
    let compiler_ver = match compiler_version_from_lockfile() {
        Some(v) => v,
        None => {
            println!(
                "cargo:warning=Phase 5: rejecting corelib {} because compiler version in \
                 Cargo.lock could not be read",
                corelib_src.display()
            );
            return false;
        }
    };
    let corelib_ver = match corelib_manifest_version(corelib_src) {
        Some(v) => v,
        None => {
            println!(
                "cargo:warning=Phase 5: rejecting corelib {} because adjacent Scarb.toml \
                 version could not be read",
                corelib_src.display()
            );
            return false;
        }
    };
    if corelib_ver != compiler_ver {
        println!(
            "cargo:warning=Phase 5: rejecting corelib {} due to version mismatch \
             (corelib={}, compiler={})",
            corelib_src.display(),
            corelib_ver,
            compiler_ver
        );
        false
    } else {
        true
    }
}

fn try_candidate(path: &Path) -> Option<PathBuf> {
    if path.exists() && corelib_layout_looks_compatible(path) && version_matches(path) {
        path.canonicalize().ok()
    } else {
        None
    }
}

fn corelib_candidate_paths() -> Vec<PathBuf> {
    let manifest_dir = match env::var("CARGO_MANIFEST_DIR") {
        Ok(value) => PathBuf::from(value),
        Err(_) => return Vec::new(),
    };
    let workspace_root = match manifest_dir.parent().and_then(|parent| parent.parent()) {
        Some(value) => value.to_path_buf(),
        None => return Vec::new(),
    };

    let mut candidates: Vec<PathBuf> = Vec::new();

    // Direct children: <workspace>/../cairo/corelib/src
    if let Some(parent) = workspace_root.parent() {
        candidates.push(parent.join("cairo/corelib/src"));

        // Sibling directories of workspace: <parent>/<sibling>/cairo/corelib/src
        // read_dir order is filesystem-dependent; sort for deterministic discovery.
        if let Ok(entries) = std::fs::read_dir(parent) {
            let mut sibling_dirs: Vec<_> = entries
                .flatten()
                .filter(|entry| entry.file_type().map(|t| t.is_dir()).unwrap_or(false))
                .collect();
            sibling_dirs.sort_by_key(|entry| entry.file_name());
            for entry in sibling_dirs {
                candidates.push(entry.path().join("cairo/corelib/src"));
                candidates.push(entry.path().join("corelib/src"));
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

    candidates.sort();
    candidates.dedup();
    candidates
}

fn emit_corelib_candidate_rerun_hints(candidates: &[PathBuf]) {
    for candidate in candidates {
        // Avoid non-existent paths: Cargo treats missing rerun-if-changed paths
        // as "always dirty", which forces build.rs to rerun every invocation.
        if !candidate.exists() {
            continue;
        }
        println!("cargo:rerun-if-changed={}", candidate.display());
        if let Some(parent) = candidate.parent() {
            let manifest = parent.join("Scarb.toml");
            if manifest.exists() {
                println!("cargo:rerun-if-changed={}", manifest.display());
            }
        }
        let lib = candidate.join("lib.cairo");
        if lib.exists() {
            println!("cargo:rerun-if-changed={}", lib.display());
        }
        let prelude = candidate.join("prelude.cairo");
        if prelude.exists() {
            println!("cargo:rerun-if-changed={}", prelude.display());
        }
        let ops = candidate.join("ops.cairo");
        if ops.exists() {
            println!("cargo:rerun-if-changed={}", ops.display());
        }
    }
}

fn find_corelib_src(candidates: &[PathBuf]) -> Option<PathBuf> {
    // 1. Explicit build-time override. If present, fail loudly when invalid.
    if let Ok(path) = env::var("UC_BUILD_CORELIB_SRC") {
        if !path.trim().is_empty() {
            let explicit = Path::new(&path);
            println!("cargo:rerun-if-changed={}", explicit.display());
            match try_candidate(explicit) {
                Some(candidate) => return Some(candidate),
                None => {
                    panic!(
                        "UC_BUILD_CORELIB_SRC is set to '{}' but it is not a compatible \
                         corelib/src directory (missing required files or version mismatch)",
                        path
                    );
                }
            }
        }
    }

    // 2. Runtime env var (also useful at build time)
    if let Ok(path) = env::var("UC_NATIVE_CORELIB_SRC") {
        if !path.trim().is_empty() {
            let runtime_override = Path::new(&path);
            println!("cargo:rerun-if-changed={}", runtime_override.display());
            if let Some(candidate) = try_candidate(runtime_override) {
                return Some(candidate);
            } else {
                println!(
                    "cargo:warning=Phase 5: UC_NATIVE_CORELIB_SRC='{}' is not a valid \
                     compatible corelib/src (missing files or version mismatch); \
                     falling back to discovery",
                    path
                );
            }
        }
    }

    // 3. Search relative to workspace root (mirrors runtime candidate search)
    for candidate in candidates {
        if let Some(p) = try_candidate(candidate) {
            return Some(p);
        }
    }

    None
}
