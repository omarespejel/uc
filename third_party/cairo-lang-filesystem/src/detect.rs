use std::path::PathBuf;

/// Detects a `corelib/src` directory using common runtime/dev search roots.
///
/// Search order:
/// 1. `CARGO_MANIFEST_DIR` ancestors (`1..=2` levels up)
/// 2. Current executable ancestors (`2..=4` levels up)
/// 3. Current working directory (`0..=0`)
///
/// Returns the first matching path ending in `corelib/src`, or `None`.
pub fn detect_corelib() -> Option<PathBuf> {
    for (base, up_options) in [
        // This is the directory of Cargo.toml of the current crate.
        // This is used for development of the compiler.
        (
            std::env::var("CARGO_MANIFEST_DIR").ok().map(PathBuf::from),
            1..=2,
        ),
        // This is the directory of the executable.
        (std::env::current_exe().ok(), 2..=4),
        // This is the current directory.
        (std::env::current_dir().ok(), 0..=0),
    ] {
        let Some(base) = base else { continue };
        for up in up_options {
            let Some(ancestor) = base.ancestors().nth(up) else {
                continue;
            };
            let mut path = ancestor.to_path_buf();
            path.push("corelib");
            path.push("src");
            if path.exists() {
                return Some(path);
            }
        }
    }
    None
}
