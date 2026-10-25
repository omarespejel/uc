use super::*;
use std::collections::BTreeSet;

/// `uc watch <address>`: poll the class hash deployed at an address, re-run the audit when it
/// changes, and report the findings that appeared or disappeared since the previous class.
#[derive(Args, Debug, Clone, Default)]
pub(crate) struct WatchArgs {
    /// Deployed contract address (`0x...`).
    pub address: String,

    /// Starknet JSON-RPC URL (else `STARKNET_RPC`, else `--network`).
    #[arg(long, visible_alias = "url")]
    pub rpc_url: Option<String>,

    /// Network preset (`mainnet`, `sepolia`, `devnet`).
    #[arg(long)]
    pub network: Option<String>,

    /// Seconds between class hash checks.
    #[arg(long, default_value_t = 60)]
    pub interval: u64,

    /// Check once and exit (for cron jobs and tests).
    #[arg(long)]
    pub once: bool,

    /// Re-audit on this check even when the class hash is unchanged.
    #[arg(long)]
    pub force: bool,

    /// Where the last class hash and findings are kept between runs
    /// (default `.uc/watch/<address>.json`).
    #[arg(long)]
    pub state: Option<PathBuf>,

    /// Decide state-dependent paths against the contract's live storage.
    #[arg(long)]
    pub live_storage: bool,

    /// Optional substring filter for function names.
    #[arg(long)]
    pub filter: Option<String>,

    /// Maximum statements followed along one symbolic path.
    #[arg(long, default_value_t = 1024)]
    pub max_depth: usize,

    /// Maximum symbolic paths explored per function.
    #[arg(long, default_value_t = 1024)]
    pub max_paths: usize,

    /// Emit one JSON document per check on stdout.
    #[arg(long)]
    pub json: bool,
}

/// What a watch remembers about the last audited class.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct WatchState {
    pub address: String,
    pub class_hash: Option<String>,
    pub block_number: Option<u64>,
    pub checked_at_epoch_ms: u64,
    /// `severity detector function` keys of the detector findings.
    pub findings: BTreeSet<String>,
    /// Functions with a satisfiable panic path.
    pub counterexamples: BTreeSet<String>,
    pub proven_safe: usize,
    pub guarded_safe: usize,
}

/// One audit of a deployed class, reduced to what the watch compares.
pub(crate) struct ClassAuditSummary {
    pub class_hash: String,
    pub findings: BTreeSet<String>,
    pub counterexamples: BTreeSet<String>,
    pub proven_safe: usize,
    pub guarded_safe: usize,
    pub functions: usize,
    pub live_storage_reads: usize,
    pub elapsed_ms: f64,
}

pub(crate) fn finding_key(f: &CaracalSierraFinding) -> String {
    format!("{} {} {}::{}", f.severity, f.detector, f.contract_or_module, f.function)
}

/// Fetches and audits the class deployed at `address`.
#[cfg(feature = "native-test-runner")]
pub(crate) fn audit_deployed_class(
    rpc: &str,
    address: &str,
    filter: Option<&str>,
    max_depth: usize,
    max_paths: usize,
    live_storage: bool,
) -> Result<ClassAuditSummary> {
    let started = Instant::now();
    let fetched = fetch_onchain_sierra_program(rpc, address)?;
    if !fetched.is_contract_address {
        bail!("`{address}` is a class hash, not a deployed contract; watch needs an address");
    }
    if fetched.class_hash.is_empty() {
        bail!("could not read the class hash deployed at {address}");
    }
    let class_hash = fetched.class_hash.clone();
    let oracle = live_storage.then(|| StorageOracle::rpc(rpc, address, 256));
    let (mut proofs, findings) = analyze_sierra_program(
        &fetched.program,
        &fetched.crate_name,
        filter,
        max_depth,
        max_paths,
        true,
        oracle.as_ref(),
    );
    let seeds = oracle.as_ref().map(|o| o.resolved()).unwrap_or_default();
    if proofs.iter().any(|p| p.status == "COUNTEREXAMPLE_FOUND") {
        replay_counterexamples_on_slice(&fetched.program, &mut proofs, &seeds);
    }
    Ok(ClassAuditSummary {
        class_hash,
        findings: findings.iter().map(finding_key).collect(),
        counterexamples: proofs
            .iter()
            .filter(|p| p.status == "COUNTEREXAMPLE_FOUND")
            .map(|p| p.function.clone())
            .collect(),
        proven_safe: proofs.iter().filter(|p| p.status == "PROVEN_SAFE").count(),
        guarded_safe: proofs.iter().filter(|p| p.status == "GUARDED_SAFE").count(),
        functions: proofs.len(),
        live_storage_reads: seeds.len(),
        elapsed_ms: started.elapsed().as_secs_f64() * 1000.0,
    })
}

fn default_state_path(address: &str) -> PathBuf {
    PathBuf::from(".uc").join("watch").join(format!("{}.json", address.trim_start_matches("0x")))
}

pub(crate) fn load_watch_state(path: &Path) -> WatchState {
    fs::read(path)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

pub(crate) fn save_watch_state(path: &Path, state: &WatchState) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, serde_json::to_vec_pretty(state)?)?;
    Ok(())
}

/// Compares a fresh audit with the remembered one. Returns the JSON document for this check
/// and the state to remember.
pub(crate) fn diff_watch(
    previous: &WatchState,
    current: &ClassAuditSummary,
    address: &str,
    block_number: Option<u64>,
) -> (serde_json::Value, WatchState) {
    let new_findings: Vec<&String> = current.findings.difference(&previous.findings).collect();
    let resolved_findings: Vec<&String> = previous.findings.difference(&current.findings).collect();
    let new_cex: Vec<&String> = current.counterexamples.difference(&previous.counterexamples).collect();
    let resolved_cex: Vec<&String> = previous.counterexamples.difference(&current.counterexamples).collect();
    let class_changed = match previous.class_hash.as_deref() {
        Some(prev) => parse_felt_str(prev).ok() != parse_felt_str(&current.class_hash).ok(),
        None => true,
    };
    let doc = serde_json::json!({
        "schema_version": UC_AGENT_JSON_SCHEMA_VERSION,
        "command": "watch",
        "address": address,
        "block_number": block_number,
        "class_hash": current.class_hash,
        "previous_class_hash": previous.class_hash,
        "class_changed": class_changed,
        "first_audit": previous.class_hash.is_none(),
        "functions": current.functions,
        "proven_safe": current.proven_safe,
        "guarded_safe": current.guarded_safe,
        "counterexamples": current.counterexamples,
        "findings": current.findings,
        "new_findings": new_findings,
        "resolved_findings": resolved_findings,
        "new_counterexamples": new_cex,
        "resolved_counterexamples": resolved_cex,
        "live_storage_reads": current.live_storage_reads,
        "audit_elapsed_ms": current.elapsed_ms,
    });
    let state = WatchState {
        address: address.to_string(),
        class_hash: Some(current.class_hash.clone()),
        block_number,
        checked_at_epoch_ms: epoch_ms_u64().unwrap_or(0),
        findings: current.findings.clone(),
        counterexamples: current.counterexamples.clone(),
        proven_safe: current.proven_safe,
        guarded_safe: current.guarded_safe,
    };
    (doc, state)
}

fn print_watch_check(doc: &serde_json::Value) {
    let s = |k: &str| doc.get(k).and_then(|v| v.as_str()).unwrap_or("?").to_string();
    let n = |k: &str| doc.get(k).and_then(|v| v.as_array()).map(|a| a.len()).unwrap_or(0);
    let list = |k: &str| -> Vec<String> {
        doc.get(k)
            .and_then(|v| v.as_array())
            .map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect())
            .unwrap_or_default()
    };
    println!(
        "{} class {}{}: {} functions, {} proven, {} guarded, {} counterexamples, {} findings ({} new, {} resolved)",
        s("address"),
        s("class_hash"),
        if doc["class_changed"].as_bool().unwrap_or(false) && !doc["first_audit"].as_bool().unwrap_or(false) {
            format!(" (changed from {})", s("previous_class_hash"))
        } else {
            String::new()
        },
        doc["functions"].as_u64().unwrap_or(0),
        doc["proven_safe"].as_u64().unwrap_or(0),
        doc["guarded_safe"].as_u64().unwrap_or(0),
        n("counterexamples"),
        n("findings"),
        n("new_findings"),
        n("resolved_findings"),
    );
    for f in list("new_findings") {
        println!("  + {f}");
    }
    for f in list("resolved_findings") {
        println!("  - {f}");
    }
    for f in list("new_counterexamples") {
        println!("  + counterexample {f}");
    }
    for f in list("resolved_counterexamples") {
        println!("  - counterexample {f}");
    }
}

pub(crate) fn run_watch(args: WatchArgs) -> Result<()> {
    #[cfg(feature = "native-test-runner")]
    {
        if !args.address.starts_with("0x") {
            bail!("watch needs a deployed contract address (0x...), got `{}`", args.address);
        }
        let rpc = resolve_read_rpc_url(args.rpc_url.as_deref(), args.network.as_deref())?;
        let state_path = args.state.clone().unwrap_or_else(|| default_state_path(&args.address));
        let mut state = load_watch_state(&state_path);
        loop {
            let class_hash_now = starknet_rpc_request(
                &rpc,
                "starknet_getClassHashAt",
                serde_json::json!({ "block_id": "latest", "contract_address": args.address }),
            )
            .ok()
            .and_then(|v| v.as_str().map(|s| format_felt_hex(&parse_felt_str(s).unwrap_or_default())));
            let block_number = starknet_rpc_request(&rpc, "starknet_blockNumber", serde_json::json!([]))
                .ok()
                .and_then(|v| v.as_u64());
            let same_hash = |a: &str, b: &str| parse_felt_str(a).ok() == parse_felt_str(b).ok();
            let unchanged = match (&class_hash_now, &state.class_hash) {
                (Some(now), Some(known)) => same_hash(now, known),
                _ => false,
            };
            if unchanged && !args.force {
                let doc = serde_json::json!({
                    "schema_version": UC_AGENT_JSON_SCHEMA_VERSION,
                    "command": "watch",
                    "address": args.address,
                    "block_number": block_number,
                    "class_hash": state.class_hash,
                    "class_changed": false,
                    "audited": false,
                });
                if args.json {
                    println!("{}", serde_json::to_string(&doc)?);
                } else {
                    println!(
                        "{} class {} unchanged at block {}",
                        args.address,
                        state.class_hash.as_deref().unwrap_or("?"),
                        block_number.map(|b| b.to_string()).unwrap_or_else(|| "?".to_string())
                    );
                }
            } else {
                let summary = audit_deployed_class(
                    &rpc,
                    &args.address,
                    args.filter.as_deref(),
                    args.max_depth,
                    args.max_paths,
                    args.live_storage,
                )?;
                let (doc, next) = diff_watch(&state, &summary, &args.address, block_number);
                if args.json {
                    println!("{}", serde_json::to_string(&doc)?);
                } else {
                    print_watch_check(&doc);
                }
                state = next;
                save_watch_state(&state_path, &state)?;
            }
            if args.once {
                return Ok(());
            }
            std::thread::sleep(Duration::from_secs(args.interval.max(1)));
        }
    }
    #[cfg(not(feature = "native-test-runner"))]
    {
        let _ = args;
        bail!("native-test-runner feature required for uc watch")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn summary(class: &str, findings: &[&str], cex: &[&str]) -> ClassAuditSummary {
        ClassAuditSummary {
            class_hash: class.to_string(),
            findings: findings.iter().map(|s| s.to_string()).collect(),
            counterexamples: cex.iter().map(|s| s.to_string()).collect(),
            proven_safe: 3,
            guarded_safe: 2,
            functions: 7,
            live_storage_reads: 0,
            elapsed_ms: 1.0,
        }
    }

    #[test]
    fn watch_diff_reports_new_and_resolved_findings_and_persists_state() {
        let first = summary("0xa", &["High unprotected-state-mutation C::set_fee", "Low cei C::swap"], &["C::withdraw"]);
        let (doc, state) = diff_watch(&WatchState::default(), &first, "0x1", Some(10));
        assert_eq!(doc["first_audit"], true);
        assert_eq!(doc["class_changed"], true);
        assert_eq!(doc["new_findings"].as_array().unwrap().len(), 2);
        assert_eq!(doc["new_counterexamples"].as_array().unwrap().len(), 1);
        assert_eq!(state.class_hash.as_deref(), Some("0xa"));

        let dir = std::env::temp_dir().join(format!("uc-watch-state-{}", std::process::id()));
        let path = dir.join("state.json");
        save_watch_state(&path, &state).unwrap();
        let reloaded = load_watch_state(&path);
        assert_eq!(reloaded.findings, state.findings);
        let _ = fs::remove_dir_all(&dir);

        let second = summary("0xb", &["Low cei C::swap", "Medium swallowed-syscall-error C::claim"], &[]);
        let (doc, state2) = diff_watch(&reloaded, &second, "0x1", Some(11));
        assert_eq!(doc["first_audit"], false);
        assert_eq!(doc["class_changed"], true);
        assert_eq!(doc["previous_class_hash"], "0xa");
        assert_eq!(doc["new_findings"], serde_json::json!(["Medium swallowed-syscall-error C::claim"]));
        assert_eq!(doc["resolved_findings"], serde_json::json!(["High unprotected-state-mutation C::set_fee"]));
        assert_eq!(doc["resolved_counterexamples"], serde_json::json!(["C::withdraw"]));
        assert_eq!(state2.class_hash.as_deref(), Some("0xb"));

        let (doc, _) = diff_watch(&state2, &second, "0x1", Some(12));
        assert_eq!(doc["class_changed"], false);
        assert!(doc["new_findings"].as_array().unwrap().is_empty());
    }

    #[test]
    fn missing_watch_state_loads_as_empty() {
        let state = load_watch_state(Path::new("/nonexistent/uc-watch/state.json"));
        assert!(state.class_hash.is_none());
        assert!(state.findings.is_empty());
    }
}
