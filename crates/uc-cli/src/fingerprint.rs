use super::*;

pub(super) fn hash_fingerprint_source_file(path: &Path) -> Result<String> {
    let is_cairo = path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.eq_ignore_ascii_case("cairo"))
        .unwrap_or(false);
    if is_cairo {
        return hash_cairo_source_semantic(path);
    }
    hash_file_blake3(path)
}

pub(super) fn hash_cairo_source_semantic(path: &Path) -> Result<String> {
    let bytes = fs::read(path).with_context(|| format!("failed to read {}", path.display()))?;
    let mut normalized = strip_cairo_comments(&bytes);
    while matches!(normalized.last(), Some(b' ' | b'\t' | b'\r' | b'\n')) {
        normalized.pop();
    }
    let mut hasher = Hasher::new();
    hasher.update(b"uc-cairo-semantic-hash-v1");
    hasher.update(&normalized);
    Ok(hasher.finalize().to_hex().to_string())
}

pub(super) fn strip_cairo_comments(input: &[u8]) -> Vec<u8> {
    #[derive(Clone, Copy)]
    enum Mode {
        Code,
        LineComment,
        BlockComment { depth: u32 },
        SingleQuote,
        DoubleQuote,
    }

    let mut mode = Mode::Code;
    let mut out = Vec::with_capacity(input.len());
    let mut i = 0_usize;

    while i < input.len() {
        let b = input[i];
        let next = input.get(i + 1).copied();

        match mode {
            Mode::Code => {
                if b == b'/' && next == Some(b'/') {
                    mode = Mode::LineComment;
                    i += 2;
                    continue;
                }
                if b == b'/' && next == Some(b'*') {
                    mode = Mode::BlockComment { depth: 1 };
                    i += 2;
                    continue;
                }
                out.push(b);
                if b == b'\'' {
                    mode = Mode::SingleQuote;
                } else if b == b'"' {
                    mode = Mode::DoubleQuote;
                }
                i += 1;
            }
            Mode::LineComment => {
                if b == b'\n' {
                    out.push(b'\n');
                    mode = Mode::Code;
                }
                i += 1;
            }
            Mode::BlockComment { depth } => {
                if b == b'/' && next == Some(b'*') {
                    mode = Mode::BlockComment { depth: depth + 1 };
                    i += 2;
                    continue;
                }
                if b == b'*' && next == Some(b'/') {
                    if depth <= 1 {
                        mode = Mode::Code;
                    } else {
                        mode = Mode::BlockComment { depth: depth - 1 };
                    }
                    i += 2;
                    continue;
                }
                if b == b'\n' {
                    out.push(b'\n');
                }
                i += 1;
            }
            Mode::SingleQuote => {
                out.push(b);
                if b == b'\\' {
                    if let Some(escaped) = next {
                        out.push(escaped);
                        i += 2;
                        continue;
                    }
                }
                if b == b'\'' {
                    mode = Mode::Code;
                }
                i += 1;
            }
            Mode::DoubleQuote => {
                out.push(b);
                if b == b'\\' {
                    if let Some(escaped) = next {
                        out.push(escaped);
                        i += 2;
                        continue;
                    }
                }
                if b == b'"' {
                    mode = Mode::Code;
                }
                i += 1;
            }
        }
    }

    out
}

pub(super) fn metadata_modified_unix_ms(metadata: &fs::Metadata) -> Result<u64> {
    let modified = metadata
        .modified()
        .context("failed to read file modification time")?;
    let since_epoch = modified.duration_since(UNIX_EPOCH).unwrap_or_default();
    u64::try_from(since_epoch.as_millis()).context("file modified time overflowed u64")
}

pub(super) fn normalize_fingerprint_path(path: &Path) -> String {
    let raw = path.to_string_lossy();
    let without_windows_prefix = raw.strip_prefix("\\\\?\\").unwrap_or(&raw);
    without_windows_prefix.replace('\\', "/")
}

pub(super) fn fingerprint_key_path(workspace_root: &Path, key: &str) -> PathBuf {
    let key_path = Path::new(key);
    if key_path.is_absolute() {
        key_path.to_path_buf()
    } else {
        workspace_root.join(key_path)
    }
}

pub(super) const MAX_EXTERNAL_PATH_DEPENDENCY_ROOTS: usize = 64;

pub(super) fn manifest_path_dependency_dirs(manifest_path: &Path) -> Result<Vec<PathBuf>> {
    let contents = fs::read_to_string(manifest_path).with_context(|| {
        format!(
            "failed to read manifest {} while collecting path dependency roots",
            manifest_path.display()
        )
    })?;
    let manifest = match toml::from_str::<TomlValue>(&contents) {
        Ok(manifest) => manifest,
        Err(err) => {
            // A manifest that is not valid TOML cannot declare path
            // dependencies scarb would honor (it is outside the build graph,
            // e.g. an intentionally-broken test fixture), and its raw bytes
            // are already part of the fingerprint, so edits still invalidate.
            eprintln!(
                "uc: warning: skipping path-dependency scan of unparseable manifest {}: {err}",
                manifest_path.display()
            );
            return Ok(Vec::new());
        }
    };
    let manifest_dir = manifest_path.parent().unwrap_or_else(|| Path::new("."));
    let mut dirs = Vec::new();
    let mut collect_table = |table: Option<&TomlValue>| {
        let Some(table) = table.and_then(TomlValue::as_table) else {
            return;
        };
        for value in table.values() {
            let Some(path) = value
                .as_table()
                .and_then(|table| table.get("path"))
                .and_then(TomlValue::as_str)
            else {
                continue;
            };
            dirs.push(manifest_dir.join(path));
        }
    };
    for section in ["dependencies", "dev-dependencies", "build-dependencies"] {
        collect_table(manifest.get(section));
    }
    collect_table(
        manifest
            .get("workspace")
            .and_then(|workspace| workspace.get("dependencies")),
    );
    Ok(dirs)
}

pub(super) fn collect_external_path_dependency_roots(
    canonical_workspace_root: &Path,
    seed_manifests: &[PathBuf],
) -> Result<Vec<PathBuf>> {
    let mut accepted: Vec<PathBuf> = Vec::new();
    let mut visited_manifests: BTreeSet<PathBuf> = BTreeSet::new();
    let mut pending: Vec<PathBuf> = seed_manifests.to_vec();
    while let Some(manifest_path) = pending.pop() {
        let canonical_manifest = match manifest_path.canonicalize() {
            Ok(path) => path,
            Err(_) => continue,
        };
        if !visited_manifests.insert(canonical_manifest.clone()) {
            continue;
        }
        for dep_dir in manifest_path_dependency_dirs(&canonical_manifest)? {
            let Ok(canonical_dir) = dep_dir.canonicalize() else {
                continue;
            };
            if canonical_dir.starts_with(canonical_workspace_root) {
                continue;
            }
            let dep_manifest = canonical_dir.join("Scarb.toml");
            if dep_manifest.is_file() {
                pending.push(dep_manifest);
            }
            if accepted.iter().any(|root| canonical_dir.starts_with(root)) {
                continue;
            }
            accepted.retain(|root| !root.starts_with(&canonical_dir));
            if accepted.len() >= MAX_EXTERNAL_PATH_DEPENDENCY_ROOTS {
                bail!(
                    "workspace references more than {MAX_EXTERNAL_PATH_DEPENDENCY_ROOTS} external path dependency roots; refusing to fingerprint"
                );
            }
            accepted.push(canonical_dir);
        }
    }
    accepted.sort();
    Ok(accepted)
}

pub(super) fn atomic_write_bytes(path: &Path, bytes: &[u8], label: &str) -> Result<()> {
    static NEXT_TEMP_ID: AtomicU64 = AtomicU64::new(1);
    let parent = path
        .parent()
        .context("cannot atomically write file without parent directory")?;
    fs::create_dir_all(parent).with_context(|| format!("failed to create {}", parent.display()))?;
    let stem = path.file_name().and_then(|v| v.to_str()).unwrap_or("file");
    let temp_id = NEXT_TEMP_ID.fetch_add(1, Ordering::Relaxed);
    let thread_id = format!("{:?}", thread::current().id());
    let temp_path = parent.join(format!(
        ".{stem}.tmp.{}.{}.{}.{}",
        std::process::id(),
        thread_id,
        temp_id,
        epoch_ms_u64().unwrap_or_default()
    ));
    fs::write(&temp_path, bytes).with_context(|| {
        format!(
            "failed to write temporary {label} file {}",
            temp_path.display()
        )
    })?;
    if let Err(err) = fs::rename(&temp_path, path) {
        let _ = fs::remove_file(&temp_path);
        return Err(err).with_context(|| {
            format!(
                "failed to move temporary {label} {} to {}",
                temp_path.display(),
                path.display()
            )
        });
    }
    Ok(())
}

pub(super) fn load_fingerprint_index(path: &Path) -> Result<FingerprintIndex> {
    if !path.exists() {
        return Ok(FingerprintIndex::empty());
    }
    let bytes = read_bytes_with_limit(path, max_fingerprint_index_bytes(), "fingerprint index")?;
    match serde_json::from_slice::<FingerprintIndex>(&bytes) {
        Ok(index) if index.schema_version == FINGERPRINT_INDEX_SCHEMA_VERSION => Ok(index),
        Ok(_) => Ok(FingerprintIndex::empty()),
        Err(err) => {
            eprintln!(
                "uc: warning: ignoring unreadable fingerprint index {}: {}",
                path.display(),
                err
            );
            Ok(FingerprintIndex::empty())
        }
    }
}

pub(super) fn save_fingerprint_index(path: &Path, index: &FingerprintIndex) -> Result<()> {
    let bytes = serde_json::to_vec(index).context("failed to encode fingerprint index")?;
    atomic_write_bytes(path, &bytes, "fingerprint index")?;
    Ok(())
}

pub(super) fn fingerprint_index_cache(
) -> &'static Mutex<HashMap<String, FingerprintIndexCacheEntry>> {
    static CACHE: OnceLock<Mutex<HashMap<String, FingerprintIndexCacheEntry>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

pub(super) fn fingerprint_index_cache_key(path: &Path) -> String {
    normalize_fingerprint_path(path)
}

pub(super) fn load_fingerprint_index_cached(path: &Path) -> Result<FingerprintIndex> {
    let key = fingerprint_index_cache_key(path);
    let now_ms = epoch_ms_u64().unwrap_or_default();
    {
        let mut cache = fingerprint_index_cache()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(entry) = cache.get_mut(&key) {
            entry.last_access_epoch_ms = now_ms;
            return Ok(entry.index.clone());
        }
    }

    let loaded = load_fingerprint_index(path)?;
    let mut cache = fingerprint_index_cache()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    cache.insert(
        key,
        FingerprintIndexCacheEntry {
            index: loaded.clone(),
            last_access_epoch_ms: now_ms,
        },
    );
    evict_oldest_fingerprint_index_cache_entries(&mut cache, fingerprint_index_cache_max_entries());
    Ok(loaded)
}

pub(super) fn store_fingerprint_index_cached(path: &Path, index: &FingerprintIndex) {
    let key = fingerprint_index_cache_key(path);
    let now_ms = epoch_ms_u64().unwrap_or_default();
    let mut cache = fingerprint_index_cache()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    cache.insert(
        key,
        FingerprintIndexCacheEntry {
            index: index.clone(),
            last_access_epoch_ms: now_ms,
        },
    );
    evict_oldest_fingerprint_index_cache_entries(&mut cache, fingerprint_index_cache_max_entries());
}

pub(super) fn evict_oldest_fingerprint_index_cache_entries(
    cache: &mut HashMap<String, FingerprintIndexCacheEntry>,
    max_entries: usize,
) {
    while cache.len() > max_entries {
        let Some(oldest_key) = cache
            .iter()
            .min_by_key(|(_, entry)| entry.last_access_epoch_ms)
            .map(|(key, _)| key.clone())
        else {
            break;
        };
        cache.remove(&oldest_key);
    }
}

pub(super) fn load_artifact_index(path: &Path) -> Result<ArtifactIndex> {
    if !path.exists() {
        return Ok(ArtifactIndex::empty());
    }
    let bytes = read_bytes_with_limit(path, max_artifact_index_bytes(), "artifact index")?;
    match serde_json::from_slice::<ArtifactIndex>(&bytes) {
        Ok(index) if index.schema_version == ARTIFACT_INDEX_SCHEMA_VERSION => Ok(index),
        Ok(_) => Ok(ArtifactIndex::empty()),
        Err(err) => {
            eprintln!(
                "uc: warning: ignoring unreadable artifact index {}: {}",
                path.display(),
                err
            );
            Ok(ArtifactIndex::empty())
        }
    }
}

pub(super) fn save_artifact_index(path: &Path, index: &ArtifactIndex) -> Result<()> {
    let bytes = serde_json::to_vec(index).context("failed to encode artifact index")?;
    atomic_write_bytes(path, &bytes, "artifact index")?;
    Ok(())
}

pub(super) fn artifact_index_cache() -> &'static Mutex<HashMap<String, ArtifactIndexCacheEntry>> {
    static CACHE: OnceLock<Mutex<HashMap<String, ArtifactIndexCacheEntry>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

pub(super) fn artifact_index_cache_key(path: &Path) -> String {
    normalize_fingerprint_path(path)
}

pub(super) fn load_artifact_index_cached(path: &Path) -> Result<ArtifactIndex> {
    let key = artifact_index_cache_key(path);
    let now_ms = epoch_ms_u64().unwrap_or_default();
    {
        let mut cache = artifact_index_cache()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(entry) = cache.get_mut(&key) {
            entry.last_access_epoch_ms = now_ms;
            return Ok(entry.index.clone());
        }
    }

    let loaded = load_artifact_index(path)?;
    let mut cache = artifact_index_cache()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    cache.insert(
        key,
        ArtifactIndexCacheEntry {
            index: loaded.clone(),
            last_access_epoch_ms: now_ms,
        },
    );
    evict_oldest_artifact_index_cache_entries(&mut cache, artifact_index_cache_max_entries());
    Ok(loaded)
}

pub(super) fn store_artifact_index_cached(path: &Path, index: &ArtifactIndex) {
    let key = artifact_index_cache_key(path);
    let now_ms = epoch_ms_u64().unwrap_or_default();
    let mut cache = artifact_index_cache()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    cache.insert(
        key,
        ArtifactIndexCacheEntry {
            index: index.clone(),
            last_access_epoch_ms: now_ms,
        },
    );
    evict_oldest_artifact_index_cache_entries(&mut cache, artifact_index_cache_max_entries());
}

pub(super) fn evict_oldest_artifact_index_cache_entries(
    cache: &mut HashMap<String, ArtifactIndexCacheEntry>,
    max_entries: usize,
) {
    while cache.len() > max_entries {
        let Some(oldest_key) = cache
            .iter()
            .min_by_key(|(_, entry)| entry.last_access_epoch_ms)
            .map(|(key, _)| key.clone())
        else {
            break;
        };
        cache.remove(&oldest_key);
    }
}

pub(super) fn compute_build_fingerprint(
    workspace_root: &Path,
    manifest_path: &Path,
    common: &BuildCommonArgs,
    profile: &str,
    cache_root: Option<&Path>,
) -> Result<String> {
    let scarb_version = scarb_version_line()?;
    compute_build_fingerprint_with_scarb_version(
        workspace_root,
        manifest_path,
        common,
        profile,
        cache_root,
        &scarb_version,
    )
}

pub(super) fn build_fingerprint_context_digest(
    manifest_identity: &str,
    common: &BuildCommonArgs,
    profile: &str,
    scarb_version: &str,
) -> String {
    let mut hasher = Hasher::new();
    hasher.update(b"uc-build-fingerprint-context-v1");
    hasher.update(scarb_version.as_bytes());
    hasher.update(current_build_env_fingerprint().as_bytes());
    hasher.update(manifest_identity.as_bytes());
    hasher.update(profile.as_bytes());
    hasher.update(common.package.as_deref().unwrap_or("*").as_bytes());
    hasher.update(if common.workspace {
        b"workspace"
    } else {
        b"package"
    });
    hasher.update(if common.offline {
        b"offline"
    } else {
        b"online"
    });
    let mut features = common.features.clone();
    features.sort_unstable();
    features.dedup();
    for feature in features {
        hasher.update(feature.as_bytes());
        hasher.update(b",");
    }
    hasher.finalize().to_hex().to_string()
}

pub(super) fn try_reuse_hot_fingerprint(
    workspace_root: &Path,
    index: &FingerprintIndex,
    context_digest: &str,
    now_ms: u64,
    mtime_recheck_window_ms: u64,
) -> Result<Option<String>> {
    if index.context_digest.as_deref() != Some(context_digest) {
        return Ok(None);
    }
    let Some(last_fingerprint) = index.last_fingerprint.as_ref() else {
        return Ok(None);
    };
    if last_fingerprint.is_empty() {
        return Ok(None);
    }

    for (relative_dir, expected_modified_unix_ms) in &index.directories {
        let dir_path = if relative_dir == "." {
            workspace_root.to_path_buf()
        } else {
            fingerprint_key_path(workspace_root, relative_dir)
        };
        let metadata = match fs::metadata(&dir_path) {
            Ok(metadata) => metadata,
            Err(_) => return Ok(None),
        };
        if !metadata.is_dir() {
            return Ok(None);
        }
        let modified_unix_ms = metadata_modified_unix_ms(&metadata)?;
        if modified_unix_ms != *expected_modified_unix_ms {
            return Ok(None);
        }
    }

    for (relative_path, cached_entry) in &index.entries {
        let file_path = fingerprint_key_path(workspace_root, relative_path);
        let metadata = match fs::metadata(&file_path) {
            Ok(metadata) => metadata,
            Err(_) => return Ok(None),
        };
        if !metadata.is_file() {
            return Ok(None);
        }
        let modified_unix_ms = metadata_modified_unix_ms(&metadata)?;
        if metadata.len() != cached_entry.size_bytes
            || modified_unix_ms != cached_entry.modified_unix_ms
        {
            return Ok(None);
        }
        if now_ms.saturating_sub(modified_unix_ms) <= mtime_recheck_window_ms {
            return Ok(None);
        }
    }

    Ok(Some(last_fingerprint.clone()))
}

pub(super) fn track_fingerprint_directories_for_relative_path(
    tracked_directories: &mut BTreeSet<String>,
    relative_path: &Path,
) {
    tracked_directories.insert(".".to_string());
    let mut cursor = relative_path.parent();
    while let Some(parent) = cursor {
        if parent.as_os_str().is_empty() {
            break;
        }
        tracked_directories.insert(normalize_fingerprint_path(parent));
        cursor = parent.parent();
    }
}

pub(super) fn track_fingerprint_directories_for_external_path(
    tracked_directories: &mut BTreeSet<String>,
    external_root: &Path,
    file_path: &Path,
) {
    let mut cursor = file_path.parent();
    while let Some(parent) = cursor {
        if !parent.starts_with(external_root) {
            break;
        }
        tracked_directories.insert(normalize_fingerprint_path(parent));
        cursor = parent.parent();
    }
}

pub(super) fn collect_fingerprint_files_from_root(
    walk_root: &Path,
    external_root: Option<&Path>,
    files: &mut Vec<(PathBuf, Option<PathBuf>)>,
    fingerprint_started: &Instant,
    fingerprint_timeout: Duration,
    max_files: usize,
) -> Result<()> {
    let walker = WalkDir::new(walk_root)
        .follow_links(false)
        .max_depth(MAX_FINGERPRINT_DEPTH)
        .into_iter()
        .filter_entry(|entry| !is_ignored_entry(walk_root, entry.path()));

    for entry in walker {
        let entry = entry.with_context(|| {
            format!(
                "failed to walk {} while collecting fingerprint files",
                walk_root.display()
            )
        })?;
        if fingerprint_started.elapsed() > fingerprint_timeout {
            bail!(
                "fingerprinting timed out after {} ms",
                fingerprint_timeout.as_millis()
            );
        }
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.path();
        if should_include_fingerprint_file(path) {
            if files.len() >= max_files {
                bail!(
                    "workspace has too many fingerprintable files (>{max_files}); refusing to hash more"
                );
            }
            files.push((path.to_path_buf(), external_root.map(Path::to_path_buf)));
        }
    }
    Ok(())
}

pub(super) fn snapshot_tracked_fingerprint_directories(
    workspace_root: &Path,
    tracked_directories: &BTreeSet<String>,
) -> Result<BTreeMap<String, u64>> {
    let mut snapshot = BTreeMap::new();
    for relative_dir in tracked_directories {
        let dir_path = if relative_dir == "." {
            workspace_root.to_path_buf()
        } else {
            fingerprint_key_path(workspace_root, relative_dir)
        };
        let metadata = fs::metadata(&dir_path)
            .with_context(|| format!("failed to stat {}", dir_path.display()))?;
        if !metadata.is_dir() {
            continue;
        }
        snapshot.insert(relative_dir.clone(), metadata_modified_unix_ms(&metadata)?);
    }
    Ok(snapshot)
}

pub(super) fn compute_build_fingerprint_with_scarb_version(
    workspace_root: &Path,
    manifest_path: &Path,
    common: &BuildCommonArgs,
    profile: &str,
    cache_root: Option<&Path>,
    scarb_version: &str,
) -> Result<String> {
    let canonical_manifest = manifest_path
        .canonicalize()
        .with_context(|| format!("failed to canonicalize {}", manifest_path.display()))?;
    let canonical_workspace_root = workspace_root
        .canonicalize()
        .with_context(|| format!("failed to canonicalize {}", workspace_root.display()))?;
    let manifest_identity = canonical_manifest
        .strip_prefix(&canonical_workspace_root)
        .map(normalize_fingerprint_path)
        .unwrap_or_else(|_| normalize_fingerprint_path(&canonical_manifest));
    let context_digest =
        build_fingerprint_context_digest(&manifest_identity, common, profile, scarb_version);

    let (index_path, mut index) = if let Some(root) = cache_root {
        let path = root.join("fingerprint/index-v1.json");
        (Some(path.clone()), load_fingerprint_index_cached(&path)?)
    } else {
        (None, FingerprintIndex::empty())
    };
    let max_files = max_fingerprint_files();
    let max_file_bytes = max_fingerprint_file_bytes();
    let max_total_bytes = max_fingerprint_total_bytes();
    let fingerprint_timeout = Duration::from_millis(fingerprint_timeout_ms());
    let fingerprint_started = Instant::now();
    let mtime_recheck_window_ms = fingerprint_mtime_recheck_window_ms();
    let now_ms = epoch_ms_u64().unwrap_or_default();

    if let Some(reused) = try_reuse_hot_fingerprint(
        workspace_root,
        &index,
        &context_digest,
        now_ms,
        mtime_recheck_window_ms,
    )? {
        return Ok(reused);
    }

    let mut hasher = Hasher::new();
    hasher.update(b"uc-build-fingerprint-v3");
    hasher.update(context_digest.as_bytes());

    let mut updated_entries: BTreeMap<String, FingerprintIndexEntry> = BTreeMap::new();
    let mut tracked_directories: BTreeSet<String> = BTreeSet::from([".".to_string()]);

    let mut files: Vec<(PathBuf, Option<PathBuf>)> = Vec::new();
    collect_fingerprint_files_from_root(
        workspace_root,
        None,
        &mut files,
        &fingerprint_started,
        fingerprint_timeout,
        max_files,
    )?;
    let mut seed_manifests: Vec<PathBuf> = files
        .iter()
        .filter(|(path, _)| path.file_name().and_then(|name| name.to_str()) == Some("Scarb.toml"))
        .map(|(path, _)| path.clone())
        .collect();
    seed_manifests.push(canonical_manifest.clone());
    let external_roots =
        collect_external_path_dependency_roots(&canonical_workspace_root, &seed_manifests)?;
    for external_root in &external_roots {
        collect_fingerprint_files_from_root(
            external_root,
            Some(external_root),
            &mut files,
            &fingerprint_started,
            fingerprint_timeout,
            max_files,
        )?;
    }
    files.sort();
    files.dedup();
    let mut total_fingerprint_bytes = 0_u64;

    for (path, external_root) in files {
        if fingerprint_started.elapsed() > fingerprint_timeout {
            bail!(
                "fingerprinting timed out after {} ms",
                fingerprint_timeout.as_millis()
            );
        }
        let rel = match external_root.as_deref() {
            None => path
                .strip_prefix(workspace_root)
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/"),
            Some(_) => normalize_fingerprint_path(&path),
        };
        match external_root.as_deref() {
            None => track_fingerprint_directories_for_relative_path(
                &mut tracked_directories,
                Path::new(&rel),
            ),
            Some(root) => track_fingerprint_directories_for_external_path(
                &mut tracked_directories,
                root,
                &path,
            ),
        }
        let metadata =
            fs::metadata(&path).with_context(|| format!("failed to stat {}", path.display()))?;
        let file_size = metadata.len();
        if file_size > max_file_bytes {
            bail!(
                "fingerprint file {} exceeds size limit ({} bytes > {} bytes)",
                path.display(),
                file_size,
                max_file_bytes
            );
        }
        total_fingerprint_bytes = total_fingerprint_bytes.saturating_add(file_size);
        if total_fingerprint_bytes > max_total_bytes {
            bail!(
                "fingerprint source budget exceeded ({} bytes > {} bytes)",
                total_fingerprint_bytes,
                max_total_bytes
            );
        }
        let modified_unix_ms = metadata_modified_unix_ms(&metadata)?;
        let file_hash = if let Some(cached) = index.entries.get(&rel) {
            let should_rehash_recent =
                now_ms.saturating_sub(modified_unix_ms) <= mtime_recheck_window_ms;
            if cached.size_bytes == file_size
                && cached.modified_unix_ms == modified_unix_ms
                && !should_rehash_recent
            {
                cached.blake3_hex.clone()
            } else {
                hash_fingerprint_source_file(&path)?
            }
        } else {
            hash_fingerprint_source_file(&path)?
        };
        updated_entries.insert(
            rel.clone(),
            FingerprintIndexEntry {
                size_bytes: file_size,
                modified_unix_ms,
                blake3_hex: file_hash.clone(),
            },
        );
        hasher.update(rel.as_bytes());
        hasher.update(b":");
        hasher.update(file_hash.as_bytes());
        hasher.update(b"\n");
    }
    let tracked_directory_mtimes =
        snapshot_tracked_fingerprint_directories(workspace_root, &tracked_directories)?;
    let fingerprint = hasher.finalize().to_hex().to_string();
    if let Some(path) = index_path {
        let changed = index.entries != updated_entries
            || index.directories != tracked_directory_mtimes
            || index.context_digest.as_deref() != Some(context_digest.as_str())
            || index.last_fingerprint.as_deref() != Some(fingerprint.as_str())
            || index.schema_version != FINGERPRINT_INDEX_SCHEMA_VERSION;
        index.schema_version = FINGERPRINT_INDEX_SCHEMA_VERSION;
        index.entries = updated_entries;
        index.directories = tracked_directory_mtimes;
        index.context_digest = Some(context_digest);
        index.last_fingerprint = Some(fingerprint.clone());
        store_fingerprint_index_cached(&path, &index);
        if changed {
            if let Err(err) = save_fingerprint_index(&path, &index) {
                eprintln!(
                    "uc: warning: failed to update fingerprint index {}: {err:#}",
                    path.display()
                );
            }
        }
    }

    Ok(fingerprint)
}

pub(super) fn is_ignored_entry(root: &Path, path: &Path) -> bool {
    if path == root {
        return false;
    }
    let Some(name) = path.file_name().and_then(|s| s.to_str()) else {
        return false;
    };
    matches!(name, ".git" | "target" | ".scarb" | ".uc")
}

pub(super) fn should_include_fingerprint_file(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|s| s.to_str()) else {
        return false;
    };

    if matches!(name, "Scarb.toml" | "Scarb.lock" | "Uc.toml") {
        return true;
    }

    path.extension()
        .and_then(|s| s.to_str())
        .map(|ext| ext == "cairo")
        .unwrap_or(false)
}
