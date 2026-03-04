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
        let mut byte = [0_u8; 1];
        let read = reader
            .read(&mut byte)
            .with_context(|| format!("failed to read {label}"))?;
        if read == 0 {
            break;
        }
        if byte[0] == b'\n' {
            break;
        }
        bytes.push(byte[0]);
        if bytes.len() > max_bytes {
            bail!("{label} exceeds size limit ({max_bytes} bytes)");
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
            DAEMON_REQUEST_SIZE_LIMIT_BYTES,
            "daemon response",
        )?
    };
    if response_line.trim().is_empty() {
        bail!("daemon returned empty response");
    }
    serde_json::from_str(response_line.trim_end()).context("failed to decode daemon response")
}

#[cfg(unix)]
pub(super) fn daemon_ping(socket_path: &Path) -> Result<DaemonStatusPayload> {
    match daemon_request(socket_path, &DaemonRequest::Ping)? {
        DaemonResponse::Pong(status) => Ok(status),
        DaemonResponse::Error { message } => bail!("daemon ping failed: {message}"),
        _ => bail!("unexpected daemon response to ping"),
    }
}

pub(super) fn daemon_request_protocol_version(request: &DaemonRequest) -> Option<&str> {
    match request {
        DaemonRequest::Build(payload) => Some(payload.protocol_version.as_str()),
        DaemonRequest::Metadata(payload) => Some(payload.protocol_version.as_str()),
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
    should_shutdown: &mut bool,
    rate_limiter: &mut DaemonRateLimiter,
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

    if !rate_limiter.allow() {
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

    let request: DaemonRequest = match serde_json::from_str(request_line.trim_end()) {
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
            DaemonResponse::Pong(daemon_status_snapshot(status, health))
        }
        DaemonRequest::Shutdown => {
            record_daemon_success(health);
            *should_shutdown = true;
            DaemonResponse::Ack
        }
        DaemonRequest::Build(request) => match execute_daemon_build(request) {
            Ok(result) => {
                record_daemon_success(health);
                DaemonResponse::Build(result)
            }
            Err(err) => {
                let message = format!("{err:#}");
                record_daemon_failure(health, message.clone());
                DaemonResponse::Error { message }
            }
        },
        DaemonRequest::Metadata(request) => match execute_daemon_metadata(request) {
            Ok(result) => {
                record_daemon_success(health);
                DaemonResponse::Metadata(result)
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
