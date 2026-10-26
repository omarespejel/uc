//! HTTP(S) for uc's network commands: JSON-RPC, Starkscan's REST API and the package registry.
//!
//! Requests go through the `curl` binary when it is installed (security tools on some machines
//! let a signed system `curl` reach the network but not a freshly built binary), else through
//! ureq over rustls with Mozilla's root certificates. With curl, the body travels in a private
//! temporary file and the headers in a config read from stdin, so neither an API key nor a large
//! body (Linux caps one argument at 128 KiB) ever appears in a process argument list.
//!
//! Responses 429 and 503 are retried with backoff (honouring `Retry-After` where the transport
//! exposes it); a connection that cannot be made is retried once over IPv4, for hosts with a
//! broken IPv6 route.

use super::*;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;
use std::time::Duration;

/// Status code and raw body of a response.
pub(crate) struct HttpResponse {
    pub status: u16,
    pub body: Vec<u8>,
}

/// Responses can be whole contract classes or traces.
const MAX_BODY_BYTES: u64 = 512 * 1024 * 1024;
const MAX_ATTEMPTS: u32 = 4;
const MAX_RETRY_WAIT: Duration = Duration::from_secs(30);
const CONNECT_TIMEOUT_SECS: u64 = 10;
const REQUEST_TIMEOUT_SECS: u64 = 300;

/// A failed exchange: `connect` when no connection was made (worth retrying over IPv4).
struct TransportError {
    connect: bool,
    message: String,
}

type Exchange = std::result::Result<(u16, Option<Duration>, Vec<u8>), TransportError>;

/// Sends one request and returns its status and body; only a transport failure is an error.
pub(crate) fn http_request(method: &str, url: &str, body: Option<&[u8]>, headers: &[(&str, &str)]) -> Result<HttpResponse> {
    let mut ipv4_only = false;
    let mut attempt = 1;
    loop {
        let exchange = match send_with_curl(method, url, body, headers, ipv4_only) {
            Some(exchange) => exchange,
            None => send_native(method, url, body, headers, ipv4_only),
        };
        match exchange {
            Ok((status, wait, _)) if matches!(status, 429 | 503) && attempt < MAX_ATTEMPTS => {
                std::thread::sleep(wait.unwrap_or(Duration::from_secs(1 << attempt)).min(MAX_RETRY_WAIT));
                attempt += 1;
            }
            Ok((status, _, body)) => return Ok(HttpResponse { status, body }),
            Err(e) if e.connect && !ipv4_only => ipv4_only = true,
            Err(e) => bail!("{method} {url} failed: {}", e.message),
        }
    }
}

/// GET that fails on any status outside 2xx (registry index and package downloads).
pub(crate) fn http_get_bytes(url: &str) -> Result<Vec<u8>> {
    let response = http_request("GET", url, None, &[])?;
    if !(200..300).contains(&response.status) {
        bail!("GET {url} returned HTTP {}", response.status);
    }
    Ok(response.body)
}

/// A curl config line, quoted as curl's config syntax wants.
fn curl_config_line(key: &str, value: &str) -> String {
    let escaped = value.replace('\\', "\\\\").replace('"', "\\\"").replace('\n', " ");
    format!("{key} = \"{escaped}\"\n")
}

/// The body in a fresh file only this user can read, removed when dropped.
struct PrivateBodyFile(PathBuf);

impl PrivateBodyFile {
    fn create(body: &[u8]) -> std::io::Result<Self> {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "uc-http-{}-{}-{}.body",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed),
            epoch_ms_u64().unwrap_or_default()
        ));
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
        options.open(&path)?.write_all(body)?;
        Ok(Self(path))
    }
}

impl Drop for PrivateBodyFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

/// `None` when curl is not installed.
fn send_with_curl(method: &str, url: &str, body: Option<&[u8]>, headers: &[(&str, &str)], ipv4_only: bool) -> Option<Exchange> {
    let failed = |message: String| Some(Err(TransportError { connect: false, message }));
    let body_file = match body.map(PrivateBodyFile::create).transpose() {
        Ok(file) => file,
        Err(e) => return failed(format!("cannot stage the request body: {e}")),
    };
    let mut config = String::new();
    for (k, v) in headers {
        config.push_str(&curl_config_line("header", &format!("{k}: {v}")));
    }
    if let Some(file) = &body_file {
        config.push_str(&curl_config_line("data-binary", &format!("@{}", file.0.display())));
    }
    config.push_str(&curl_config_line("url", url));
    let mut cmd = Command::new("curl");
    cmd.args(["-sS", "-L", "-X", method, "-K", "-", "-w", "\n%{http_code}"])
        .args(["--connect-timeout", &CONNECT_TIMEOUT_SECS.to_string(), "--max-time", &REQUEST_TIMEOUT_SECS.to_string()])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    if ipv4_only {
        cmd.arg("-4");
    }
    let mut child = match cmd.spawn() {
        Ok(child) => child,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return None,
        Err(e) => return failed(format!("cannot run curl: {e}")),
    };
    if let Some(mut stdin) = child.stdin.take() {
        if let Err(e) = stdin.write_all(config.as_bytes()) {
            return failed(format!("cannot pass the request to curl: {e}"));
        }
    }
    let output = match child.wait_with_output() {
        Ok(output) => output,
        Err(e) => return failed(format!("curl did not finish: {e}")),
    };
    if !output.status.success() {
        // 6: host not resolved, 7: no connection, 28: timed out.
        let connect = matches!(output.status.code(), Some(6 | 7 | 28));
        return Some(Err(TransportError { connect, message: String::from_utf8_lossy(&output.stderr).trim().to_string() }));
    }
    let stdout = output.stdout;
    let split = stdout.iter().rposition(|b| *b == b'\n').unwrap_or(0);
    let status = String::from_utf8_lossy(&stdout[split..]).trim().parse().unwrap_or(0);
    Some(Ok((status, None, stdout[..split].to_vec())))
}

fn native_agent(ipv4_only: bool) -> &'static ureq::Agent {
    static ANY: OnceLock<ureq::Agent> = OnceLock::new();
    static IPV4: OnceLock<ureq::Agent> = OnceLock::new();
    let cell = if ipv4_only { &IPV4 } else { &ANY };
    cell.get_or_init(|| {
        let family = if ipv4_only { ureq::config::IpFamily::Ipv4Only } else { ureq::config::IpFamily::Any };
        ureq::Agent::config_builder()
            .http_status_as_error(false)
            .timeout_connect(Some(Duration::from_secs(CONNECT_TIMEOUT_SECS)))
            .timeout_global(Some(Duration::from_secs(REQUEST_TIMEOUT_SECS)))
            .ip_family(family)
            .user_agent(concat!("uc/", env!("CARGO_PKG_VERSION")))
            .build()
            .new_agent()
    })
}

fn send_native(method: &str, url: &str, body: Option<&[u8]>, headers: &[(&str, &str)], ipv4_only: bool) -> Exchange {
    let agent = native_agent(ipv4_only);
    let sent = match method {
        "GET" | "DELETE" => {
            let mut req = if method == "GET" { agent.get(url) } else { agent.delete(url) };
            for (k, v) in headers {
                req = req.header(*k, *v);
            }
            req.call()
        }
        _ => {
            let mut req = if method == "PUT" { agent.put(url) } else { agent.post(url) };
            for (k, v) in headers {
                req = req.header(*k, *v);
            }
            req.send(body.unwrap_or_default())
        }
    };
    let transport = |e: ureq::Error| TransportError {
        connect: matches!(e, ureq::Error::ConnectionFailed | ureq::Error::HostNotFound | ureq::Error::Io(_) | ureq::Error::Timeout(_)),
        message: e.to_string(),
    };
    let mut response = sent.map_err(transport)?;
    let status = response.status().as_u16();
    let wait = response
        .headers()
        .get("retry-after")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.trim().parse::<u64>().ok())
        .map(Duration::from_secs);
    let bytes = response.body_mut().with_config().limit(MAX_BODY_BYTES).read_to_vec().map_err(transport)?;
    Ok((status, wait, bytes))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Read};
    use std::net::TcpListener;

    /// A one-shot local server that answers 429 once, then echoes the request body; it records
    /// the header lines it saw.
    fn serve(listener: TcpListener, seen: std::sync::Arc<std::sync::Mutex<Vec<String>>>) {
        for (i, stream) in listener.incoming().take(2).enumerate() {
            let mut stream = stream.unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut length = 0usize;
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                if line == "\r\n" || line.is_empty() {
                    break;
                }
                if let Some(v) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                    length = v.trim().parse().unwrap();
                }
                seen.lock().unwrap().push(line.trim().to_string());
            }
            let mut body = vec![0u8; length];
            reader.read_exact(&mut body).unwrap();
            let reply = if i == 0 {
                "HTTP/1.1 429 Too Many Requests\r\nRetry-After: 0\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".as_bytes().to_vec()
            } else {
                let mut r = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len()).into_bytes();
                r.extend_from_slice(&body);
                r
            };
            stream.write_all(&reply).unwrap();
        }
    }

    #[test]
    fn large_bodies_and_secret_headers_go_through_and_429_is_retried() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/rpc", listener.local_addr().unwrap());
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let server = std::thread::spawn({
            let seen = seen.clone();
            move || serve(listener, seen)
        });
        // Larger than the 128 KiB Linux allows in one argument.
        let body: Vec<u8> = (0..300_000).map(|i| b'a' + (i % 26) as u8).collect();
        let response = http_request("POST", &url, Some(&body), &[("X-Starkscan-Api-Key", "secret \"key\"")]).unwrap();
        server.join().unwrap();
        assert_eq!(response.status, 200);
        assert_eq!(response.body, body, "the body survives the round trip");
        assert!(seen.lock().unwrap().iter().any(|l| l == "X-Starkscan-Api-Key: secret \"key\""), "{:?}", seen.lock().unwrap());
    }
}
