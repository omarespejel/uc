use super::*;

const DEFAULT_CACHE_OBJECT_HASH_MEMO_MAX_ENTRIES: usize = 4096;

#[derive(Clone)]
pub(super) struct BuildEntryCacheEntry {
    pub(super) file_size_bytes: u64,
    pub(super) file_modified_unix_ms: u64,
    pub(super) entry: BuildCacheEntry,
    pub(super) last_access_epoch_ms: u64,
}

#[derive(Clone)]
pub(super) struct CacheObjectHashMemoEntry {
    pub(super) size_bytes: u64,
    pub(super) modified_unix_ms: u64,
    pub(super) blake3_hex: String,
    pub(super) last_access_epoch_ms: u64,
}

pub(super) fn async_persist_scope_key(workspace_root: &Path, profile: &str) -> String {
    format!("{}::{profile}", workspace_root.display())
}

pub(super) fn async_persist_in_flight_set() -> &'static Mutex<HashSet<String>> {
    static IN_FLIGHT: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    IN_FLIGHT.get_or_init(|| Mutex::new(HashSet::new()))
}

pub(super) fn try_mark_async_persist_in_flight(scope_key: &str) -> bool {
    let mut in_flight = async_persist_in_flight_set()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    in_flight.insert(scope_key.to_string())
}

pub(super) fn clear_async_persist_in_flight(scope_key: &str) {
    let mut in_flight = async_persist_in_flight_set()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    in_flight.remove(scope_key);
}

pub(super) fn async_persist_error_slot() -> &'static Mutex<VecDeque<String>> {
    static SLOT: OnceLock<Mutex<VecDeque<String>>> = OnceLock::new();
    SLOT.get_or_init(|| Mutex::new(VecDeque::new()))
}

fn async_persist_error_queue_limit() -> usize {
    static VALUE: OnceLock<usize> = OnceLock::new();
    *VALUE.get_or_init(|| {
        parse_env_usize(
            "UC_ASYNC_PERSIST_ERROR_QUEUE_LIMIT",
            ASYNC_PERSIST_ERROR_QUEUE_LIMIT,
        )
        .max(1)
    })
}

fn async_persist_error_log_path() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("UC_ASYNC_PERSIST_ERROR_LOG_PATH") {
        let path = PathBuf::from(path);
        if !path.as_os_str().is_empty() {
            return Some(path);
        }
    }
    let home = std::env::var_os("HOME")?;
    Some(PathBuf::from(home).join(".uc/cache/persist-errors.log"))
}

pub(super) fn maybe_rotate_async_persist_error_log(path: &Path, max_bytes: u64) -> io::Result<()> {
    if max_bytes == 0 {
        return Ok(());
    }
    let metadata = match fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(err) => return Err(err),
    };
    if metadata.len() < max_bytes {
        return Ok(());
    }

    let rotated = PathBuf::from(format!("{}.1", path.display()));
    let _ = fs::remove_file(&rotated);
    fs::rename(path, rotated)
}

fn append_async_persist_error_log(message: &str) {
    let Some(path) = async_persist_error_log_path() else {
        return;
    };
    let Some(parent) = path.parent() else {
        return;
    };
    if let Err(err) = fs::create_dir_all(parent) {
        eprintln!(
            "uc: warning: failed to create async persist error log dir {}: {err}",
            parent.display()
        );
        return;
    }

    let max_bytes = async_persist_error_log_max_bytes();
    if let Err(err) = maybe_rotate_async_persist_error_log(&path, max_bytes) {
        eprintln!(
            "uc: warning: failed to rotate async persist error log {}: {err}",
            path.display()
        );
    }

    let mut file = match OpenOptions::new().create(true).append(true).open(&path) {
        Ok(file) => file,
        Err(err) => {
            eprintln!(
                "uc: warning: failed to open async persist error log {}: {err}",
                path.display()
            );
            return;
        }
    };
    let now_ms = epoch_ms_u64().unwrap_or_default();
    if let Err(err) = writeln!(file, "{now_ms}\t{}", message.replace('\n', "\\n")) {
        eprintln!(
            "uc: warning: failed to write async persist error log {}: {err}",
            path.display()
        );
    }
}

pub(super) fn record_async_persist_error(error: String) {
    append_async_persist_error_log(&error);
    let queue_limit = async_persist_error_queue_limit();
    let mut slot = async_persist_error_slot()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    slot.push_back(error);
    while slot.len() > queue_limit {
        if let Some(dropped) = slot.pop_front() {
            append_async_persist_error_log(&format!("queue_drop oldest={dropped}"));
            tracing::warn!(
                dropped_error = %dropped,
                "async cache persistence error queue dropped oldest entry"
            );
            eprintln!(
                "uc: warning: async cache persistence error queue dropped oldest entry: {dropped}"
            );
        }
    }
}

pub(super) fn take_async_persist_errors() -> Vec<String> {
    let mut slot = async_persist_error_slot()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    slot.drain(..).collect()
}

pub(super) struct AsyncPersistGuard {
    pub(super) scope_key: String,
}

impl AsyncPersistGuard {
    fn new(scope_key: String) -> Self {
        Self { scope_key }
    }
}

impl Drop for AsyncPersistGuard {
    fn drop(&mut self) {
        clear_async_persist_in_flight(&self.scope_key);
    }
}

pub(super) struct AsyncPersistTask {
    pub(super) scope_key: String,
    pub(super) workspace_root: PathBuf,
    pub(super) profile: String,
    pub(super) fingerprint: String,
    pub(super) cache_root: PathBuf,
    pub(super) objects_dir: PathBuf,
    pub(super) entry_path: PathBuf,
}

pub(super) fn async_persist_sender() -> &'static SyncSender<AsyncPersistTask> {
    static SENDER: OnceLock<SyncSender<AsyncPersistTask>> = OnceLock::new();
    SENDER.get_or_init(|| {
        let (sender, receiver) = mpsc::sync_channel(ASYNC_PERSIST_QUEUE_LIMIT);
        thread::spawn(move || run_async_persist_worker(receiver));
        sender
    })
}

pub(super) fn run_async_persist_worker(receiver: Receiver<AsyncPersistTask>) {
    for task in receiver {
        let _guard = AsyncPersistGuard::new(task.scope_key.clone());
        if let Err(err) = persist_cache_entry_for_build(
            &task.workspace_root,
            &task.profile,
            &task.fingerprint,
            &task.cache_root,
            &task.objects_dir,
            &task.entry_path,
        ) {
            let _ = fs::remove_file(&task.entry_path);
            record_async_persist_error(err.to_string());
            tracing::warn!(error = %format!("{err:#}"), "async cache persistence failed");
            eprintln!("uc: warning: async cache persistence failed: {err:#}");
        }
    }
}

pub(super) fn persist_cache_entry(
    profile: &str,
    fingerprint: &str,
    cached_artifacts: &[CachedArtifact],
    entry_path: &Path,
) -> Result<()> {
    let entry = BuildCacheEntry {
        schema_version: BUILD_CACHE_SCHEMA_VERSION,
        fingerprint: fingerprint.to_string(),
        profile: profile.to_string(),
        artifacts: cached_artifacts.to_vec(),
    };

    let bytes = serde_json::to_vec(&entry)?;
    atomic_write_bytes(entry_path, &bytes, "cache entry")?;
    if let Ok(metadata) = fs::metadata(entry_path) {
        store_build_entry_cached(entry_path, &metadata, &entry);
    }

    Ok(())
}

pub(super) fn persist_cache_entry_for_build(
    workspace_root: &Path,
    profile: &str,
    fingerprint: &str,
    cache_root: &Path,
    objects_dir: &Path,
    entry_path: &Path,
) -> Result<()> {
    let _ = persist_cache_entry_for_build_with_artifacts(
        workspace_root,
        profile,
        fingerprint,
        cache_root,
        objects_dir,
        entry_path,
    )?;
    Ok(())
}

pub(super) fn persist_cache_entry_for_build_with_artifacts(
    workspace_root: &Path,
    profile: &str,
    fingerprint: &str,
    cache_root: &Path,
    objects_dir: &Path,
    entry_path: &Path,
) -> Result<Vec<CachedArtifact>> {
    let cached_artifacts =
        collect_cached_artifacts_for_entry(workspace_root, profile, cache_root, objects_dir)?;
    let _cache_lock = acquire_cache_lock(cache_root)?;
    persist_cache_entry(profile, fingerprint, &cached_artifacts, entry_path)?;
    if should_enforce_cache_size_budget_now() {
        enforce_cache_size_budget(cache_root)?;
    }
    Ok(cached_artifacts)
}

pub(super) fn enforce_cache_size_budget(cache_root: &Path) -> Result<()> {
    enforce_cache_size_budget_with_budget(cache_root, max_cache_bytes())
}

pub(super) fn enforce_cache_size_budget_with_budget(cache_root: &Path, budget: u64) -> Result<()> {
    if budget == 0 || !cache_root.exists() {
        return Ok(());
    }

    #[derive(Clone)]
    struct CacheFile {
        path: PathBuf,
        size: u64,
        modified_ms: u64,
        is_object: bool,
    }

    let mut files = Vec::<CacheFile>::new();
    let mut total = 0_u64;
    for entry in WalkDir::new(cache_root).follow_links(false).into_iter() {
        let entry = entry.with_context(|| {
            format!(
                "failed to traverse cache tree under {}",
                cache_root.display()
            )
        })?;
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.path();
        if !is_removable_cache_file(path) {
            continue;
        }
        let metadata =
            fs::metadata(path).with_context(|| format!("failed to stat {}", path.display()))?;
        let size = metadata.len();
        total = total.saturating_add(size);
        let modified_ms = metadata_modified_unix_ms(&metadata).unwrap_or_default();
        let is_object = path
            .components()
            .any(|c| matches!(c, Component::Normal(name) if name == "objects"));
        files.push(CacheFile {
            path: path.to_path_buf(),
            size,
            modified_ms,
            is_object,
        });
    }

    if total <= budget {
        if total > (budget.saturating_mul(9) / 10) {
            eprintln!("uc: cache usage is high: {total} / {budget} bytes");
        }
        return Ok(());
    }

    files.sort_by(|a, b| {
        a.is_object
            .cmp(&b.is_object)
            .reverse()
            .then_with(|| a.modified_ms.cmp(&b.modified_ms))
            .then_with(|| a.path.cmp(&b.path))
    });

    let mut removed = 0_u64;
    for file in files {
        if total <= budget {
            break;
        }
        match fs::remove_file(&file.path) {
            Ok(()) => {
                total = total.saturating_sub(file.size);
                removed = removed.saturating_add(file.size);
            }
            Err(err) if err.kind() == io::ErrorKind::NotFound => {
                total = total.saturating_sub(file.size);
            }
            Err(err) => {
                eprintln!(
                    "uc: warning: failed to evict cache file {}: {err}",
                    file.path.display()
                );
            }
        }
    }

    if removed > 0 {
        eprintln!(
            "uc: cache eviction removed {} bytes (budget {} bytes)",
            removed, budget
        );
    }
    if total > budget {
        eprintln!(
            "uc: warning: cache remains over budget after eviction ({} > {} bytes)",
            total, budget
        );
    }
    Ok(())
}

pub(super) fn is_removable_cache_file(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|v| v.to_str()) else {
        return false;
    };
    name != ".lock"
}

fn cache_object_hash_memo_max_entries() -> usize {
    static VALUE: OnceLock<usize> = OnceLock::new();
    *VALUE.get_or_init(|| {
        parse_env_usize(
            "UC_CACHE_OBJECT_HASH_MEMO_MAX_ENTRIES",
            DEFAULT_CACHE_OBJECT_HASH_MEMO_MAX_ENTRIES,
        )
        .max(1)
    })
}

fn cache_object_hash_memo() -> &'static Mutex<HashMap<String, CacheObjectHashMemoEntry>> {
    static CACHE: OnceLock<Mutex<HashMap<String, CacheObjectHashMemoEntry>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn cache_object_hash_memo_key(path: &Path) -> String {
    normalize_fingerprint_path(path)
}

pub(super) fn evict_oldest_cache_object_hash_memo_entries(
    cache: &mut HashMap<String, CacheObjectHashMemoEntry>,
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

fn cached_object_hash_if_fresh(
    object_path: &Path,
    metadata: &fs::Metadata,
) -> Result<Option<String>> {
    let modified_unix_ms = metadata_modified_unix_ms(metadata)?;
    let now_ms = epoch_ms_u64().unwrap_or_default();
    let key = cache_object_hash_memo_key(object_path);
    let mut cache = cache_object_hash_memo()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let Some(entry) = cache.get_mut(&key) else {
        return Ok(None);
    };
    entry.last_access_epoch_ms = now_ms;
    if entry.size_bytes == metadata.len() && entry.modified_unix_ms == modified_unix_ms {
        return Ok(Some(entry.blake3_hex.clone()));
    }
    Ok(None)
}

fn store_cache_object_hash(object_path: &Path, metadata: &fs::Metadata, hash: &str) -> Result<()> {
    let modified_unix_ms = metadata_modified_unix_ms(metadata)?;
    let now_ms = epoch_ms_u64().unwrap_or_default();
    let key = cache_object_hash_memo_key(object_path);
    let mut cache = cache_object_hash_memo()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    cache.insert(
        key,
        CacheObjectHashMemoEntry {
            size_bytes: metadata.len(),
            modified_unix_ms,
            blake3_hex: hash.to_ascii_lowercase(),
            last_access_epoch_ms: now_ms,
        },
    );
    evict_oldest_cache_object_hash_memo_entries(&mut cache, cache_object_hash_memo_max_entries());
    Ok(())
}

pub(super) fn build_entry_cache() -> &'static Mutex<HashMap<String, BuildEntryCacheEntry>> {
    static CACHE: OnceLock<Mutex<HashMap<String, BuildEntryCacheEntry>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

pub(super) fn build_entry_cache_key(path: &Path) -> String {
    normalize_fingerprint_path(path)
}

pub(super) fn remove_build_entry_cached(path: &Path) {
    let key = build_entry_cache_key(path);
    let mut cache = build_entry_cache()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    cache.remove(&key);
}

pub(super) fn store_build_entry_cached(
    path: &Path,
    metadata: &fs::Metadata,
    entry: &BuildCacheEntry,
) {
    let Ok(file_modified_unix_ms) = metadata_modified_unix_ms(metadata) else {
        return;
    };
    let key = build_entry_cache_key(path);
    let now_ms = epoch_ms_u64().unwrap_or_default();
    let mut cache = build_entry_cache()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    cache.insert(
        key,
        BuildEntryCacheEntry {
            file_size_bytes: metadata.len(),
            file_modified_unix_ms,
            entry: entry.clone(),
            last_access_epoch_ms: now_ms,
        },
    );
    evict_oldest_build_entry_cache_entries(&mut cache, build_entry_cache_max_entries());
}

pub(super) fn evict_oldest_build_entry_cache_entries(
    cache: &mut HashMap<String, BuildEntryCacheEntry>,
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

pub(super) fn load_cache_entry_cached(path: &Path) -> Result<Option<BuildCacheEntry>> {
    if !path.exists() {
        remove_build_entry_cached(path);
        return Ok(None);
    }
    let metadata =
        fs::metadata(path).with_context(|| format!("failed to stat {}", path.display()))?;
    let max_bytes = max_cache_entry_bytes();
    if metadata.len() > max_bytes {
        remove_build_entry_cached(path);
        eprintln!(
            "uc: warning: ignoring oversized cache entry {} ({} bytes > {} bytes)",
            path.display(),
            metadata.len(),
            max_bytes
        );
        return Ok(None);
    }
    let file_modified_unix_ms = metadata_modified_unix_ms(&metadata)?;
    let key = build_entry_cache_key(path);
    let now_ms = epoch_ms_u64().unwrap_or_default();
    {
        let mut cache = build_entry_cache()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(entry) = cache.get_mut(&key) {
            entry.last_access_epoch_ms = now_ms;
            if entry.file_size_bytes == metadata.len()
                && entry.file_modified_unix_ms == file_modified_unix_ms
            {
                return Ok(Some(entry.entry.clone()));
            }
        }
    }
    let loaded = load_cache_entry(path)?;
    let mut cache = build_entry_cache()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    match loaded.as_ref() {
        Some(entry) => {
            cache.insert(
                key,
                BuildEntryCacheEntry {
                    file_size_bytes: metadata.len(),
                    file_modified_unix_ms,
                    entry: entry.clone(),
                    last_access_epoch_ms: now_ms,
                },
            );
            evict_oldest_build_entry_cache_entries(&mut cache, build_entry_cache_max_entries());
        }
        None => {
            cache.remove(&key);
        }
    }
    Ok(loaded)
}

pub(super) fn load_cache_entry(path: &Path) -> Result<Option<BuildCacheEntry>> {
    if !path.exists() {
        return Ok(None);
    }

    let metadata =
        fs::metadata(path).with_context(|| format!("failed to stat {}", path.display()))?;
    let max_bytes = max_cache_entry_bytes();
    if metadata.len() > max_bytes {
        eprintln!(
            "uc: warning: ignoring oversized cache entry {} ({} bytes > {} bytes)",
            path.display(),
            metadata.len(),
            max_bytes
        );
        return Ok(None);
    }
    let file = File::open(path).with_context(|| format!("failed to read {}", path.display()))?;
    let mut reader = BufReader::new(file).take(max_bytes + 1);
    let mut bytes = Vec::new();
    reader
        .read_to_end(&mut bytes)
        .with_context(|| format!("failed to read {}", path.display()))?;
    if bytes.len() as u64 > max_bytes {
        eprintln!(
            "uc: warning: ignoring oversized cache entry {} (>{} bytes)",
            path.display(),
            max_bytes
        );
        return Ok(None);
    }
    let parsed: BuildCacheEntry = match serde_json::from_slice(&bytes) {
        Ok(entry) => entry,
        Err(err) => {
            eprintln!(
                "uc: warning: ignoring unreadable cache entry {}: {}",
                path.display(),
                err
            );
            return Ok(None);
        }
    };
    Ok(Some(parsed))
}

pub(super) fn artifact_index_entry_matches_expected(
    index_entry: &ArtifactIndexEntry,
    metadata: &fs::Metadata,
    expected_hash: &str,
    expected_size: u64,
) -> Result<bool> {
    let modified_unix_ms = metadata_modified_unix_ms(metadata)?;
    Ok(index_entry.size_bytes == expected_size
        && index_entry.size_bytes == metadata.len()
        && index_entry.modified_unix_ms == modified_unix_ms
        && index_entry.blake3_hex == expected_hash)
}

pub(super) fn upsert_artifact_index_entry_from_metadata(
    index: &mut ArtifactIndex,
    relative_path: &str,
    metadata: &fs::Metadata,
    expected_hash: &str,
) -> Result<()> {
    let modified_unix_ms = metadata_modified_unix_ms(metadata)?;
    index.entries.insert(
        relative_path.to_string(),
        ArtifactIndexEntry {
            size_bytes: metadata.len(),
            modified_unix_ms,
            blake3_hex: expected_hash.to_string(),
        },
    );
    Ok(())
}

pub(super) fn cache_object_matches_expected(
    object_path: &Path,
    expected_hash: &str,
    expected_size: u64,
) -> Result<bool> {
    let metadata = match fs::metadata(object_path) {
        Ok(metadata) => metadata,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(err) => {
            return Err(err).with_context(|| format!("failed to stat {}", object_path.display()))
        }
    };
    if !metadata.is_file() || metadata.len() != expected_size {
        return Ok(false);
    }
    let actual_hash =
        if let Some(cached_hash) = cached_object_hash_if_fresh(object_path, &metadata)? {
            cached_hash
        } else {
            let computed = hash_file_blake3(object_path)?;
            store_cache_object_hash(object_path, &metadata, &computed)?;
            computed
        };
    Ok(actual_hash.eq_ignore_ascii_case(expected_hash))
}

pub(super) fn restore_cached_artifacts(
    workspace_root: &Path,
    profile: &str,
    cache_root: &Path,
    objects_dir: &Path,
    artifacts: &[CachedArtifact],
) -> Result<bool> {
    if artifacts.is_empty() {
        return Ok(true);
    }

    let index_path = cache_root.join("artifact-index-v1.json");
    let mut artifact_index = load_artifact_index_cached(&index_path)?;
    let mut artifact_index_changed = false;
    let target_root = workspace_root.join("target").join(profile);

    for artifact in artifacts {
        validate_hex_digest(
            "cached artifact blake3 hash",
            &artifact.blake3_hex,
            MIN_HASH_LEN,
        )?;
        validate_cache_object_rel_path(&artifact.object_rel_path)?;
        let expected_hash = artifact.blake3_hex.as_str();
        let relative_path = validated_relative_artifact_path(&artifact.relative_path)?;
        let relative_path_key = normalize_fingerprint_path(&relative_path);
        let out_path = target_root.join(&relative_path);
        ensure_path_within_root(&target_root, &out_path, "cache restore path")?;

        if let Ok(existing_metadata) = fs::metadata(&out_path) {
            let mut matches_cached_artifact = false;
            if existing_metadata.is_file() {
                if let Some(index_entry) = artifact_index.entries.get(&relative_path_key) {
                    if artifact_index_entry_matches_expected(
                        index_entry,
                        &existing_metadata,
                        expected_hash,
                        artifact.size_bytes,
                    )? {
                        matches_cached_artifact = true;
                    }
                }
                if !matches_cached_artifact
                    && existing_metadata.len() == artifact.size_bytes
                    && existing_metadata.len() <= max_restore_existing_hash_bytes()
                    && hash_file_blake3(&out_path)? == *expected_hash
                {
                    upsert_artifact_index_entry_from_metadata(
                        &mut artifact_index,
                        &relative_path_key,
                        &existing_metadata,
                        expected_hash,
                    )?;
                    artifact_index_changed = true;
                    matches_cached_artifact = true;
                }
            }
            if matches_cached_artifact {
                continue;
            }
        }

        let object_path = objects_dir.join(&artifact.object_rel_path);
        ensure_path_within_root(objects_dir, &object_path, "cache object path")?;
        if !object_path.exists() {
            return Ok(false);
        }
        if !cache_object_matches_expected(&object_path, expected_hash, artifact.size_bytes)? {
            eprintln!(
                "uc: warning: cache object integrity mismatch for {}; evicting object and treating as cache miss",
                object_path.display()
            );
            let _ = fs::remove_file(&object_path);
            return Ok(false);
        }
        if let Some(parent) = out_path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }

        restore_cache_object(&object_path, &out_path).with_context(|| {
            format!(
                "failed to restore cache object {} to {}",
                object_path.display(),
                out_path.display()
            )
        })?;
        let restored_metadata = fs::metadata(&out_path)
            .with_context(|| format!("failed to stat {}", out_path.display()))?;
        upsert_artifact_index_entry_from_metadata(
            &mut artifact_index,
            &relative_path_key,
            &restored_metadata,
            expected_hash,
        )?;
        artifact_index_changed = true;
    }

    if artifact_index_changed {
        artifact_index.schema_version = ARTIFACT_INDEX_SCHEMA_VERSION;
        store_artifact_index_cached(&index_path, &artifact_index);
        if let Err(err) = save_artifact_index(&index_path, &artifact_index) {
            eprintln!(
                "uc: warning: failed to update artifact index {}: {err:#}",
                index_path.display()
            );
        }
    }

    Ok(true)
}

pub(super) fn cached_artifacts_already_materialized(
    workspace_root: &Path,
    profile: &str,
    cache_root: &Path,
    artifacts: &[CachedArtifact],
) -> Result<bool> {
    if artifacts.is_empty() {
        return Ok(true);
    }

    let index_path = cache_root.join("artifact-index-v1.json");
    let artifact_index = load_artifact_index_cached(&index_path)?;
    let target_root = workspace_root.join("target").join(profile);

    for artifact in artifacts {
        validate_hex_digest(
            "cached artifact blake3 hash",
            &artifact.blake3_hex,
            MIN_HASH_LEN,
        )?;
        let relative_path = validated_relative_artifact_path(&artifact.relative_path)?;
        let relative_path_key = normalize_fingerprint_path(&relative_path);
        let Some(index_entry) = artifact_index.entries.get(&relative_path_key) else {
            return Ok(false);
        };
        let out_path = target_root.join(&relative_path);
        ensure_path_within_root(&target_root, &out_path, "cache materialized check path")?;
        let metadata = match fs::metadata(&out_path) {
            Ok(metadata) => metadata,
            Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(false),
            Err(err) => {
                return Err(err).with_context(|| format!("failed to stat {}", out_path.display()))
            }
        };
        if !metadata.is_file() {
            return Ok(false);
        }
        if !artifact_index_entry_matches_expected(
            index_entry,
            &metadata,
            &artifact.blake3_hex,
            artifact.size_bytes,
        )? {
            return Ok(false);
        }
    }

    Ok(true)
}

pub(super) fn try_reflink_file(source: &Path, destination: &Path) -> io::Result<()> {
    #[cfg(target_os = "linux")]
    {
        let source_file = File::open(source)?;
        let destination_file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(destination)?;
        let result = unsafe {
            libc::ioctl(
                destination_file.as_raw_fd(),
                libc::FICLONE as _,
                source_file.as_raw_fd(),
            )
        };
        if result == 0 {
            return Ok(());
        }
        Err(io::Error::last_os_error())
    }

    #[cfg(target_os = "macos")]
    {
        let source_c = CString::new(source.as_os_str().as_bytes())
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "source path contains NUL"))?;
        let destination_c = CString::new(destination.as_os_str().as_bytes()).map_err(|_| {
            io::Error::new(io::ErrorKind::InvalidInput, "destination path contains NUL")
        })?;
        let result = unsafe { libc::clonefile(source_c.as_ptr(), destination_c.as_ptr(), 0) };
        if result == 0 {
            return Ok(());
        }
        Err(io::Error::last_os_error())
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = (source, destination);
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "reflink is not supported on this platform",
        ))
    }
}

pub(super) fn restore_cache_object(source: &Path, destination: &Path) -> Result<()> {
    if destination.exists() {
        fs::remove_file(destination)
            .with_context(|| format!("failed to replace {}", destination.display()))?;
    }

    if let Err(reflink_err) = try_reflink_file(source, destination) {
        let _ = fs::remove_file(destination);
        match fs::hard_link(source, destination) {
            Ok(()) => return Ok(()),
            Err(link_err) => {
                fs::copy(source, destination).with_context(|| {
                    format!(
                        "failed to copy cache object after reflink ({}) and hard-link ({}) fallbacks: {} -> {}",
                        reflink_err,
                        link_err,
                        source.display(),
                        destination.display()
                    )
                })?;
            }
        }
    }
    Ok(())
}

pub(super) fn hash_file_blake3(path: &Path) -> Result<String> {
    let metadata =
        fs::metadata(path).with_context(|| format!("failed to stat {}", path.display()))?;
    if metadata.len() > MAX_CACHEABLE_ARTIFACT_BYTES {
        bail!(
            "file {} exceeds hashing size limit ({} bytes > {} bytes)",
            path.display(),
            metadata.len(),
            MAX_CACHEABLE_ARTIFACT_BYTES
        );
    }
    let file =
        fs::File::open(path).with_context(|| format!("failed to open {}", path.display()))?;
    let mut reader = BufReader::new(file);
    let mut buf = [0_u8; 8192];
    let mut hasher = Hasher::new();

    loop {
        let read = reader
            .read(&mut buf)
            .with_context(|| format!("failed to read {}", path.display()))?;
        if read == 0 {
            break;
        }
        hasher.update(&buf[..read]);
    }

    Ok(hasher.finalize().to_hex().to_string())
}
