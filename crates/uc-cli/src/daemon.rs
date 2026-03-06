use super::*;

pub(super) fn daemon_socket_path(override_path: Option<PathBuf>) -> Result<PathBuf> {
    if let Some(path) = override_path {
        return Ok(path);
    }
    if let Some(path) = std::env::var_os("UC_DAEMON_SOCKET_PATH") {
        return Ok(PathBuf::from(path));
    }
    let home = std::env::var_os("HOME").context("HOME is not set; provide --socket-path")?;
    Ok(PathBuf::from(home).join(".uc/daemon/uc.sock"))
}

pub(super) fn daemon_log_path(socket_path: &Path) -> PathBuf {
    socket_path.with_extension("log")
}

pub(super) fn remove_socket_if_exists(path: &Path) -> Result<()> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(err) => {
            return Err(err).with_context(|| format!("failed to stat {}", path.display()));
        }
    };

    #[cfg(unix)]
    {
        use std::os::unix::fs::FileTypeExt;
        if !metadata.file_type().is_socket() {
            bail!(
                "refusing to remove non-socket path {}; provide a unix socket path",
                path.display()
            );
        }
    }

    #[cfg(not(unix))]
    {
        if !metadata.file_type().is_file() {
            bail!(
                "refusing to remove non-file path {}; provide a socket file path",
                path.display()
            );
        }
    }

    fs::remove_file(path).with_context(|| format!("failed to remove {}", path.display()))
}

pub(super) fn open_daemon_log_file(log_path: &Path) -> Result<(File, File)> {
    let log_file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path)
        .with_context(|| format!("failed to open daemon log {}", log_path.display()))?;
    #[cfg(unix)]
    {
        fs::set_permissions(log_path, fs::Permissions::from_mode(0o600)).with_context(|| {
            format!(
                "failed to set daemon log permissions for {}",
                log_path.display()
            )
        })?;
    }
    let log_file_err = log_file
        .try_clone()
        .with_context(|| format!("failed to clone log file {}", log_path.display()))?;
    Ok((log_file, log_file_err))
}

pub(super) fn rotate_daemon_log_if_needed(log_path: &Path) -> Result<()> {
    let metadata = match fs::metadata(log_path) {
        Ok(metadata) => metadata,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(err) => {
            return Err(err).with_context(|| format!("failed to stat {}", log_path.display()));
        }
    };
    if metadata.len() < DAEMON_LOG_ROTATE_BYTES {
        return Ok(());
    }
    let rotated = PathBuf::from(format!("{}.1", log_path.display()));
    if rotated.exists() {
        fs::remove_file(&rotated)
            .with_context(|| format!("failed to remove {}", rotated.display()))?;
    }
    fs::rename(log_path, &rotated).with_context(|| {
        format!(
            "failed to rotate daemon log {} to {}",
            log_path.display(),
            rotated.display()
        )
    })?;
    Ok(())
}

pub(super) fn read_line_limited<R: BufRead>(
    reader: &mut R,
    max_bytes: usize,
    label: &str,
) -> Result<String> {
    let mut bytes = Vec::with_capacity(128);
    loop {
        let chunk = reader
            .fill_buf()
            .with_context(|| format!("failed to read {label}"))?;
        if chunk.is_empty() {
            break;
        }

        let newline_pos = chunk.iter().position(|byte| *byte == b'\n');
        let take_len = newline_pos.unwrap_or(chunk.len());
        let remaining = max_bytes.saturating_sub(bytes.len());
        // Allow reading exactly `max_bytes` bytes (without newline) so callers can
        // accept payloads up to the documented limit. If no newline is found,
        // the next non-empty read will fail once `remaining == 0`.
        if take_len > remaining {
            bail!("{label} exceeds size limit ({max_bytes} bytes)");
        }
        bytes.extend_from_slice(&chunk[..take_len]);

        let consumed = if newline_pos.is_some() {
            take_len + 1
        } else {
            take_len
        };
        reader.consume(consumed);
        if newline_pos.is_some() {
            break;
        }
    }
    if bytes.is_empty() {
        return Ok(String::new());
    }
    String::from_utf8(bytes).with_context(|| format!("{label} is not valid UTF-8"))
}

#[cfg(unix)]
pub(super) fn daemon_request(
    socket_path: &Path,
    request: &DaemonRequest,
) -> Result<DaemonResponse> {
    daemon_request_with_timeouts(
        socket_path,
        request,
        daemon_request_read_timeout(request),
        daemon_request_write_timeout(),
    )
}

fn debug_daemon_response_enabled() -> bool {
    static VALUE: OnceLock<bool> = OnceLock::new();
    *VALUE.get_or_init(|| {
        matches!(
            std::env::var("UC_DEBUG_DAEMON_RESPONSE")
                .ok()
                .map(|value| value.trim().to_ascii_lowercase()),
            Some(value) if matches!(value.as_str(), "1" | "true" | "yes" | "on")
        )
    })
}

fn flatten_payload_wrapped_wire_shape(value: &mut serde_json::Value) {
    let Some(root) = value.as_object_mut() else {
        return;
    };
    let Some(payload) = root.remove("payload") else {
        return;
    };
    let serde_json::Value::Object(payload_map) = payload else {
        root.insert("payload".to_string(), payload);
        return;
    };
    for (key, item) in payload_map {
        if key == "type" {
            continue;
        }
        // Skip redundant payload `type`: the root discriminant is authoritative and
        // must not be overridden by payload-wrapped clients.
        // Non-discriminant payload fields stay authoritative over root duplicates.
        root.insert(key, item);
    }
}

#[cfg(unix)]
pub(super) fn daemon_request_with_timeouts(
    socket_path: &Path,
    request: &DaemonRequest,
    read_timeout: Option<Duration>,
    write_timeout: Option<Duration>,
) -> Result<DaemonResponse> {
    let mut stream = UnixStream::connect(socket_path)
        .with_context(|| format!("failed to connect daemon socket {}", socket_path.display()))?;
    stream
        .set_read_timeout(read_timeout)
        .with_context(|| format!("failed to set read timeout for {}", socket_path.display()))?;
    stream
        .set_write_timeout(write_timeout)
        .with_context(|| format!("failed to set write timeout for {}", socket_path.display()))?;

    let payload = serde_json::to_vec(request).context("failed to encode daemon request")?;
    stream
        .write_all(&payload)
        .context("failed to write daemon request payload")?;
    stream
        .write_all(b"\n")
        .context("failed to write daemon request newline")?;
    stream.flush().context("failed to flush daemon request")?;

    let response_line = {
        let mut reader = BufReader::new(&mut stream);
        read_line_limited(
            &mut reader,
            daemon_response_size_limit_bytes(),
            "daemon response",
        )?
    };
    if response_line.trim().is_empty() {
        bail!("daemon returned empty response");
    }
    match decode_daemon_response(response_line.trim_end()) {
        Ok(response) => Ok(response),
        Err(err) => {
            if debug_daemon_response_enabled() {
                eprintln!(
                    "uc: debug raw daemon response: {}",
                    response_line.trim_end()
                );
            }
            Err(err).context("failed to decode daemon response")
        }
    }
}

// Daemon wire payloads can be wrapped (`payload`) or flat (legacy/hybrid).
// Normalize through `Value` once and decode from the normalized shape.
pub(super) fn decode_daemon_request(line: &str) -> serde_json::Result<DaemonRequest> {
    let mut value: serde_json::Value = serde_json::from_str(line)?;
    flatten_payload_wrapped_wire_shape(&mut value);
    serde_json::from_value(value)
}

// Daemon wire payloads can be wrapped (`payload`) or flat (legacy/hybrid).
// Normalize through `Value` once and decode from the normalized shape.
pub(super) fn decode_daemon_response(line: &str) -> serde_json::Result<DaemonResponse> {
    let mut value: serde_json::Value = serde_json::from_str(line)?;
    flatten_payload_wrapped_wire_shape(&mut value);
    serde_json::from_value(value)
}

#[cfg(unix)]
pub(super) fn daemon_ping(socket_path: &Path) -> Result<DaemonStatusPayload> {
    match daemon_request(socket_path, &DaemonRequest::Ping)? {
        DaemonResponse::Pong { payload } => Ok(payload),
        DaemonResponse::Error { message } => bail!("daemon ping failed: {message}"),
        _ => bail!("unexpected daemon response to ping"),
    }
}

pub(super) fn daemon_request_protocol_version(request: &DaemonRequest) -> Option<&str> {
    match request {
        DaemonRequest::Build { payload } => Some(payload.protocol_version.as_str()),
        DaemonRequest::Metadata { payload } => Some(payload.protocol_version.as_str()),
        DaemonRequest::Ping | DaemonRequest::Shutdown => None,
    }
}

pub(super) fn validate_daemon_request_protocol_version(request: &DaemonRequest) -> Result<()> {
    let Some(version) = daemon_request_protocol_version(request) else {
        return Ok(());
    };
    validate_daemon_protocol_version(version).context("daemon request protocol mismatch")
}

#[cfg(unix)]
pub(super) fn daemon_status_snapshot(
    base: &DaemonStatusPayload,
    health: &Arc<Mutex<DaemonHealth>>,
) -> DaemonStatusPayload {
    let snapshot = health
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone();
    let native_cache = native_cache_telemetry_snapshot();
    DaemonStatusPayload {
        pid: base.pid,
        started_at_epoch_ms: base.started_at_epoch_ms,
        socket_path: base.socket_path.clone(),
        protocol_version: base.protocol_version.clone(),
        healthy: snapshot.consecutive_failures < 3,
        total_requests: snapshot.total_requests,
        failed_requests: snapshot.failed_requests,
        rate_limited_requests: snapshot.rate_limited_requests,
        last_error: snapshot.last_error,
        native_compile_session_cache_entries: native_cache.session_entries,
        native_compile_session_cache_estimated_bytes: native_cache.session_estimated_bytes,
        native_compile_context_cache_entries: native_cache.context_entries,
        native_compile_context_cache_estimated_bytes: native_cache.context_estimated_bytes,
        native_compile_session_build_locks: native_cache.build_locks,
        metadata_result_cache_entries: native_cache.metadata_entries,
        metadata_result_cache_estimated_bytes: native_cache.metadata_estimated_bytes,
        native_refresh_none_count: native_cache.refresh_none_count,
        native_refresh_incremental_count: native_cache.refresh_incremental_count,
        native_refresh_full_rebuild_count: native_cache.refresh_full_rebuild_count,
        native_refresh_changed_files_total: native_cache.refresh_changed_files_total,
        native_refresh_removed_files_total: native_cache.refresh_removed_files_total,
        native_fallback_preflight_ineligible_count: native_cache
            .fallback_preflight_ineligible_count,
        native_fallback_local_native_error_count: native_cache.fallback_local_native_error_count,
        native_fallback_daemon_native_error_count: native_cache.fallback_daemon_native_error_count,
        native_fallback_daemon_backend_downgrade_count: native_cache
            .fallback_daemon_backend_downgrade_count,
    }
}

#[cfg(unix)]
pub(super) fn record_daemon_success(health: &Arc<Mutex<DaemonHealth>>) {
    let mut state = health
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    state.total_requests = state.total_requests.saturating_add(1);
    state.consecutive_failures = 0;
    state.last_error = None;
    state.last_failure_at = None;
}

#[cfg(unix)]
pub(super) fn record_daemon_failure(health: &Arc<Mutex<DaemonHealth>>, error: String) {
    let mut state = health
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    state.total_requests = state.total_requests.saturating_add(1);
    state.failed_requests = state.failed_requests.saturating_add(1);
    state.consecutive_failures = state.consecutive_failures.saturating_add(1);
    state.last_error = Some(error);
    state.last_failure_at = Some(Instant::now());
}

#[cfg(unix)]
pub(super) fn record_daemon_rate_limit(health: &Arc<Mutex<DaemonHealth>>) {
    let mut state = health
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    state.total_requests = state.total_requests.saturating_add(1);
    state.failed_requests = state.failed_requests.saturating_add(1);
    state.rate_limited_requests = state.rate_limited_requests.saturating_add(1);
    state.consecutive_failures = state.consecutive_failures.saturating_add(1);
    state.last_error = Some("daemon rate limit exceeded; retry shortly".to_string());
    state.last_failure_at = Some(Instant::now());
}

#[cfg(unix)]
pub(super) fn maybe_auto_recover_daemon_health(health: &Arc<Mutex<DaemonHealth>>) {
    let mut state = health
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if state.consecutive_failures < 3 {
        return;
    }
    let Some(last_failure_at) = state.last_failure_at else {
        return;
    };
    if last_failure_at.elapsed() >= Duration::from_secs(DAEMON_UNHEALTHY_RECOVERY_SECONDS) {
        state.consecutive_failures = 0;
        state.last_error = None;
        state.last_failure_at = None;
    }
}

#[cfg(unix)]
pub(super) fn handle_daemon_connection(
    mut stream: UnixStream,
    status: &DaemonStatusPayload,
    health: &Arc<Mutex<DaemonHealth>>,
    should_shutdown: &Arc<AtomicBool>,
    rate_limiter: &Arc<Mutex<DaemonRateLimiter>>,
) -> Result<()> {
    stream
        .set_read_timeout(Some(Duration::from_secs(30)))
        .context("failed to set daemon read timeout")?;
    stream
        .set_write_timeout(Some(Duration::from_secs(120)))
        .context("failed to set daemon write timeout")?;

    let request_line = {
        let mut reader = BufReader::new(&mut stream);
        read_line_limited(
            &mut reader,
            DAEMON_REQUEST_SIZE_LIMIT_BYTES,
            "daemon request",
        )?
    };
    if request_line.trim().is_empty() {
        return Ok(());
    }
    maybe_auto_recover_daemon_health(health);

    let allowed = {
        let mut limiter = rate_limiter
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        limiter.allow()
    };
    if !allowed {
        record_daemon_rate_limit(health);
        let response = DaemonResponse::Error {
            message: "daemon rate limit exceeded; retry shortly".to_string(),
        };
        let payload = serde_json::to_vec(&response).context("failed to encode daemon response")?;
        stream
            .write_all(&payload)
            .context("failed to write daemon response")?;
        stream
            .write_all(b"\n")
            .context("failed to write daemon response newline")?;
        stream.flush().context("failed to flush daemon response")?;
        return Ok(());
    }

    let request: DaemonRequest = match decode_daemon_request(request_line.trim_end()) {
        Ok(request) => request,
        Err(err) => {
            let message = format!("failed to parse daemon request: {err}");
            record_daemon_failure(health, message.clone());
            let response = DaemonResponse::Error { message };
            let payload =
                serde_json::to_vec(&response).context("failed to encode daemon response")?;
            stream
                .write_all(&payload)
                .context("failed to write daemon response")?;
            stream
                .write_all(b"\n")
                .context("failed to write daemon response newline")?;
            stream.flush().context("failed to flush daemon response")?;
            return Ok(());
        }
    };

    if let Err(err) = validate_daemon_request_protocol_version(&request) {
        let message = format!("{err:#}");
        record_daemon_failure(health, message.clone());
        let response = DaemonResponse::Error { message };
        let payload = serde_json::to_vec(&response).context("failed to encode daemon response")?;
        stream
            .write_all(&payload)
            .context("failed to write daemon response")?;
        stream
            .write_all(b"\n")
            .context("failed to write daemon response newline")?;
        stream.flush().context("failed to flush daemon response")?;
        return Ok(());
    }

    let response = match request {
        DaemonRequest::Ping => {
            record_daemon_success(health);
            DaemonResponse::Pong {
                payload: daemon_status_snapshot(status, health),
            }
        }
        DaemonRequest::Shutdown => {
            record_daemon_success(health);
            should_shutdown.store(true, Ordering::Release);
            DaemonResponse::Ack
        }
        DaemonRequest::Build { payload } => match execute_daemon_build(payload) {
            Ok(result) => {
                record_daemon_success(health);
                DaemonResponse::Build { payload: result }
            }
            Err(err) => {
                let message = format!("{err:#}");
                record_daemon_failure(health, message.clone());
                DaemonResponse::Error { message }
            }
        },
        DaemonRequest::Metadata { payload } => match execute_daemon_metadata(payload) {
            Ok(result) => {
                record_daemon_success(health);
                DaemonResponse::Metadata { payload: result }
            }
            Err(err) => {
                let message = format!("{err:#}");
                record_daemon_failure(health, message.clone());
                DaemonResponse::Error { message }
            }
        },
    };

    let payload = serde_json::to_vec(&response).context("failed to encode daemon response")?;
    stream
        .write_all(&payload)
        .context("failed to write daemon response")?;
    stream
        .write_all(b"\n")
        .context("failed to write daemon response newline")?;
    stream.flush().context("failed to flush daemon response")?;
    Ok(())
}
