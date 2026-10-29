//! `uc doctor`: the checks that decide whether uc's network and build commands can work on this
//! machine. The toolchain and cache, the HTTPS transport, RPC reachability over IPv4 and IPv6,
//! whether this binary may open connections itself, the Starkscan key, and the external tools
//! uc can fall back to.

use super::*;
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

#[derive(Args, Debug)]
pub(crate) struct DoctorArgs {
    /// Skip the network checks.
    #[arg(long)]
    pub offline: bool,
    #[arg(long)]
    pub json: bool,
}

/// One check: `ok == false` only for something that stops uc from working.
struct Check {
    name: String,
    ok: bool,
    detail: String,
    fix: Option<String>,
}

impl Check {
    fn pass(name: impl Into<String>, detail: impl Into<String>) -> Self {
        Self { name: name.into(), ok: true, detail: detail.into(), fix: None }
    }
    fn fail(name: impl Into<String>, detail: impl Into<String>, fix: impl Into<String>) -> Self {
        Self { name: name.into(), ok: false, detail: detail.into(), fix: Some(fix.into()) }
    }
}

fn first_line(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).lines().next().unwrap_or_default().trim().to_string()
}

/// `<tool> <version>` from the first line of `--version` (curl's goes on with its libraries).
fn tool_version(tool: &str) -> Option<String> {
    let out = Command::new(tool).arg("--version").output().ok()?;
    out.status.success().then(|| first_line(&out.stdout).split_whitespace().take(2).collect::<Vec<_>>().join(" "))
}

fn cache_check() -> Check {
    let Some(dir) = native_global_cache_root_dir() else {
        return Check::fail("cache", "no HOME to place ~/.uc/cache in", "set UC_GLOBAL_CACHE_DIR to a writable directory");
    };
    let probe = dir.join(format!(".doctor-{}", std::process::id()));
    let writable = fs::create_dir_all(&dir).is_ok() && fs::write(&probe, b"ok").is_ok();
    let _ = fs::remove_file(&probe);
    if writable {
        Check::pass("cache", format!("{} is writable", dir.display()))
    } else {
        Check::fail("cache", format!("{} is not writable", dir.display()), "set UC_GLOBAL_CACHE_DIR to a writable directory")
    }
}

/// Whether curl reaches `url` over one IP family (any HTTP status counts as reached).
fn curl_reaches(url: &str, family: &str) -> Option<bool> {
    let out = Command::new("curl").args([family, "-sS", "-o", "/dev/null", "-m", "8", url]).output().ok()?;
    Some(out.status.success())
}

/// Whether this process can open a TCP connection itself (some security tools allow a signed
/// system curl and block freshly built binaries).
fn direct_connect(host: &str) -> bool {
    let Ok(mut addrs) = (host, 443).to_socket_addrs() else { return false };
    addrs.any(|a| TcpStream::connect_timeout(&a, Duration::from_secs(3)).is_ok())
}

fn network_checks(checks: &mut Vec<Check>) {
    for network in ["mainnet", "sepolia"] {
        let Some(url) = default_rpc_url_for_network(network) else { continue };
        let started = Instant::now();
        let name = format!("rpc {network}");
        match starknet_rpc_request(&url, "starknet_chainId", serde_json::json!([])) {
            Ok(chain) => checks.push(Check::pass(
                name,
                format!("{url} answers chain id {} in {} ms", chain.as_str().unwrap_or("?"), started.elapsed().as_millis()),
            )),
            Err(e) => checks.push(Check::fail(name, format!("{url}: {e:#}"), "check the network, or pass --url with another RPC endpoint")),
        }
    }
    let url = "https://api.cartridge.gg/x/starknet/mainnet";
    if let (Some(v4), Some(v6)) = (curl_reaches(url, "-4"), curl_reaches(url, "-6")) {
        let detail = match (v4, v6) {
            (true, true) => "IPv4 and IPv6 both reach the RPC host".to_string(),
            (true, false) => "IPv6 does not reach the RPC host; uc retries over IPv4".to_string(),
            (false, true) => "only IPv6 reaches the RPC host".to_string(),
            (false, false) => "neither IPv4 nor IPv6 reaches the RPC host".to_string(),
        };
        checks.push(if v4 || v6 { Check::pass("ip routes", detail) } else { Check::fail("ip routes", detail, "check the network connection or proxy") });
    }
    let detail = if direct_connect("api.cartridge.gg") {
        "uc can open connections itself".to_string()
    } else {
        "uc cannot open connections itself (a security tool or sandbox blocks this binary); requests go through curl".to_string()
    };
    checks.push(Check::pass("direct connections", detail));
    checks.push(match starkscan_api_key() {
        None => Check::pass("starkscan", "STARKSCAN_API_KEY not set: --network uses the public Cartridge endpoints"),
        Some(_) => match starknet_rpc_request(&starkscan_rpc_url("SN_MAIN"), "starknet_chainId", serde_json::json!([])) {
            Ok(_) => Check::pass("starkscan", "STARKSCAN_API_KEY is accepted by the Starkscan gateway"),
            Err(e) => Check::fail("starkscan", format!("the gateway refused the key: {e:#}"), "check STARKSCAN_API_KEY or unset it to use the public endpoints"),
        },
    });
}

pub(crate) fn run_doctor(args: DoctorArgs) -> Result<()> {
    let mut checks = vec![Check::pass(
        "toolchain",
        format!("uc {} with Cairo {} embedded", env!("CARGO_PKG_VERSION"), native_cairo_lang_compiler_version()),
    )];
    checks.push(cache_check());
    checks.push(Check::pass(
        "https transport",
        match tool_version("curl") {
            Some(v) => format!("curl ({v}); bodies and headers never reach the command line"),
            None => "built in (ureq over rustls); curl is not installed".to_string(),
        },
    ));
    if !args.offline {
        network_checks(&mut checks);
    }
    for tool in ["scarb", "snforge", "sncast", "starknet-devnet"] {
        let detail = tool_version(tool).unwrap_or_else(|| "not installed (uc does not need it)".to_string());
        checks.push(Check::pass(format!("{tool} (optional)"), detail));
    }
    let failed = checks.iter().filter(|c| !c.ok).count();
    if args.json {
        let rows: Vec<serde_json::Value> = checks
            .iter()
            .map(|c| serde_json::json!({ "check": c.name, "ok": c.ok, "detail": c.detail, "fix": c.fix }))
            .collect();
        json_println!("{}", serde_json::to_string_pretty(&serde_json::json!({ "schema_version": UC_AGENT_JSON_SCHEMA_VERSION, "ok": failed == 0, "checks": rows }))?);
    } else {
        for c in &checks {
            println!("{} {:<26} {}", if c.ok { "ok  " } else { "FAIL" }, c.name, c.detail);
            if let Some(fix) = &c.fix {
                println!("     {:<26} fix: {fix}", "");
            }
        }
    }
    if failed > 0 {
        bail!("{failed} check(s) failed");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn doctor_offline_passes_on_a_machine_with_a_writable_cache() {
        run_doctor(DoctorArgs { offline: true, json: true }).expect("offline doctor");
        assert!(cache_check().ok);
        assert_eq!(Check::fail("x", "y", "z").fix.as_deref(), Some("z"));
    }
}
