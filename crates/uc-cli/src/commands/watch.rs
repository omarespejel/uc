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

/// Audits an already fetched class (used by `uc watch` and the declared-class feed).
#[cfg(feature = "native-test-runner")]
pub(crate) fn audit_fetched_class(
    fetched: &OnchainSierraProgram,
    filter: Option<&str>,
    max_depth: usize,
    max_paths: usize,
    oracle: Option<&StorageOracle>,
) -> ClassAuditSummary {
    let started = Instant::now();
    let (mut proofs, findings) = analyze_sierra_program(
        &fetched.program,
        &fetched.crate_name,
        filter,
        max_depth,
        max_paths,
        true,
        oracle,
    );
    let seeds = oracle.map(|o| o.resolved()).unwrap_or_default();
    if proofs.iter().any(|p| p.status == "COUNTEREXAMPLE_FOUND") {
        replay_counterexamples_on_slice(&fetched.program, &mut proofs, &seeds);
    }
    ClassAuditSummary {
        class_hash: fetched.class_hash.clone(),
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
    }
}

/// Letter grade shared with `uc audit`: A+ nothing found, A low findings only, B high findings,
/// C satisfiable panic paths.
pub(crate) fn grade_for(counterexamples: usize, findings: &BTreeSet<String>) -> &'static str {
    let high = findings.iter().filter(|f| f.starts_with("High ") || f.starts_with("Critical ")).count();
    if counterexamples == 0 && high == 0 {
        if findings.is_empty() { "A+" } else { "A" }
    } else if counterexamples == 0 {
        "B"
    } else {
        "C"
    }
}

/// `uc audit --declared-since <block>`: every Sierra class declared in a block range, audited
/// in declaration order. Cairo 0 (deprecated) declarations are listed but not analysed.
#[cfg(feature = "native-test-runner")]
pub(crate) fn run_declared_class_feed(args: &AuditArgs) -> Result<()> {
    let started = Instant::now();
    let rpc = resolve_read_rpc_url(args.rpc_url.as_deref(), Some(args.network.as_deref().unwrap_or("mainnet")))?;
    let from = args.declared_since.ok_or_else(|| anyhow!("--declared-since <block> is required"))?;
    let latest = starknet_rpc_request(&rpc, "starknet_blockNumber", serde_json::json!([]))?
        .as_u64()
        .ok_or_else(|| anyhow!("starknet_blockNumber returned no number"))?;
    let to = args.declared_until.unwrap_or(latest).min(latest);
    if from > to {
        bail!("--declared-since {from} is after the last block {to}");
    }
    let mut classes: Vec<serde_json::Value> = Vec::new();
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut blocks_scanned = 0u64;
    'blocks: for block in from..=to {
        blocks_scanned += 1;
        let update = starknet_rpc_request(
            &rpc,
            "starknet_getStateUpdate",
            serde_json::json!({ "block_id": { "block_number": block } }),
        )
        .with_context(|| format!("failed to fetch the state update of block {block}"))?;
        let diff = update.get("state_diff").cloned().unwrap_or_default();
        let declared: Vec<String> = diff
            .get("declared_classes")
            .and_then(|v| v.as_array())
            .map(|a| a.iter().filter_map(|d| d.get("class_hash").and_then(|h| h.as_str()).map(str::to_string)).collect())
            .unwrap_or_default();
        let deprecated: Vec<String> = diff
            .get("deprecated_declared_classes")
            .and_then(|v| v.as_array())
            .map(|a| a.iter().filter_map(|h| h.as_str().map(str::to_string)).collect())
            .unwrap_or_default();
        for class_hash in deprecated {
            if seen.insert(class_hash.clone()) {
                classes.push(serde_json::json!({
                    "block_number": block,
                    "class_hash": class_hash,
                    "kind": "cairo0",
                    "audited": false,
                    "reason": "Cairo 0 classes cannot be analysed",
                }));
            }
        }
        for class_hash in declared {
            if !seen.insert(class_hash.clone()) {
                continue;
            }
            if classes.iter().filter(|c| c["audited"] == true).count() >= args.max_classes {
                break 'blocks;
            }
            let entry = match fetch_onchain_sierra_program(&rpc, &class_hash) {
                Ok(fetched) => {
                    let summary = audit_fetched_class(&fetched, args.filter.as_deref(), args.max_depth, args.max_paths, None);
                    serde_json::json!({
                        "block_number": block,
                        "class_hash": class_hash,
                        "kind": "sierra",
                        "audited": true,
                        "grade": grade_for(summary.counterexamples.len(), &summary.findings),
                        "sierra_statements": fetched.program.statements.len(),
                        "functions": summary.functions,
                        "proven_safe": summary.proven_safe,
                        "guarded_safe": summary.guarded_safe,
                        "counterexamples": summary.counterexamples,
                        "findings": summary.findings,
                        "elapsed_ms": summary.elapsed_ms,
                    })
                }
                Err(e) => serde_json::json!({
                    "block_number": block,
                    "class_hash": class_hash,
                    "kind": "sierra",
                    "audited": false,
                    "reason": e.to_string(),
                }),
            };
            if !args.json && args.report_path.is_none() {
                print_feed_entry(&entry);
            }
            classes.push(entry);
        }
    }
    let report = serde_json::json!({
        "schema_version": UC_AGENT_JSON_SCHEMA_VERSION,
        "command": "audit-declared",
        "rpc_url": rpc,
        "rpc_provider": rpc_provider_label(&rpc),
        "from_block": from,
        "to_block": to,
        "blocks_scanned": blocks_scanned,
        "classes_declared": classes.len(),
        "classes_audited": classes.iter().filter(|c| c["audited"] == true).count(),
        "classes": classes,
        "elapsed_ms": started.elapsed().as_secs_f64() * 1000.0,
    });
    if args.json || args.report_path.is_some() {
        emit_json_value(args.report_path.as_deref(), &report)?;
    } else {
        println!(
            "blocks {from}..={to}: {} classes declared, {} audited, {:.1} s",
            report["classes_declared"],
            report["classes_audited"],
            report["elapsed_ms"].as_f64().unwrap_or(0.0) / 1000.0
        );
    }
    Ok(())
}

fn print_feed_entry(entry: &serde_json::Value) {
    let block = entry["block_number"].as_u64().unwrap_or(0);
    let hash = entry["class_hash"].as_str().unwrap_or("?");
    if entry["audited"] == true {
        println!(
            "block {block} class {hash}: grade {} | {} functions, {} proven, {} guarded, {} counterexamples, {} findings",
            entry["grade"].as_str().unwrap_or("?"),
            entry["functions"],
            entry["proven_safe"],
            entry["guarded_safe"],
            entry["counterexamples"].as_array().map(|a| a.len()).unwrap_or(0),
            entry["findings"].as_array().map(|a| a.len()).unwrap_or(0),
        );
        for f in entry["findings"].as_array().into_iter().flatten().filter_map(|f| f.as_str()) {
            println!("    {f}");
        }
    } else {
        println!("block {block} class {hash}: skipped ({})", entry["reason"].as_str().unwrap_or("?"));
    }
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
    let oracle = live_storage.then(|| StorageOracle::rpc(rpc, address, 256));
    let mut summary = audit_fetched_class(&fetched, filter, max_depth, max_paths, oracle.as_ref());
    summary.elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;
    Ok(summary)
}

fn default_state_path(address: &str) -> PathBuf {
    let norm = parse_felt_str(address)
        .map(|f| format_felt_hex(&f))
        .unwrap_or_else(|_| address.to_string());
    PathBuf::from(".uc")
        .join("watch")
        .join(format!("{}.json", norm.trim_start_matches("0x")))
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
                .and_then(|v| {
                    v.as_str()
                        .and_then(|s| parse_felt_str(s).ok().map(|f| format_felt_hex(&f)))
                });
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

    /// A one-thread HTTP JSON-RPC stub answering the methods the declared-class feed uses.
    #[cfg(feature = "native-test-runner")]
    fn spawn_fake_rpc(class_json: serde_json::Value, class_hash: &str) -> String {
        use std::io::{BufRead, BufReader, Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let class_hash = class_hash.to_string();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { break };
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut content_length = 0usize;
                loop {
                    let mut line = String::new();
                    if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" || line == "\n" {
                        break;
                    }
                    if let Some(v) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                        content_length = v.trim().parse().unwrap_or(0);
                    }
                }
                let mut body = vec![0u8; content_length];
                reader.read_exact(&mut body).unwrap();
                let req: serde_json::Value = serde_json::from_slice(&body).unwrap_or_default();
                let method = req["method"].as_str().unwrap_or("");
                let result = match method {
                    "starknet_blockNumber" => serde_json::json!(102),
                    "starknet_getStateUpdate" => {
                        let block = req["params"]["block_id"]["block_number"].as_u64().unwrap_or(0);
                        if block == 101 {
                            serde_json::json!({ "state_diff": {
                                "declared_classes": [{ "class_hash": class_hash, "compiled_class_hash": "0x1" }],
                                "deprecated_declared_classes": ["0xdeadc0de"]
                            }})
                        } else {
                            serde_json::json!({ "state_diff": { "declared_classes": [], "deprecated_declared_classes": [] } })
                        }
                    }
                    "starknet_getClassAt" => serde_json::Value::Null,
                    "starknet_getClass" => class_json.clone(),
                    _ => serde_json::Value::Null,
                };
                let response = if result.is_null() {
                    serde_json::json!({ "jsonrpc": "2.0", "id": 1, "error": { "code": 20, "message": "not found" } })
                } else {
                    serde_json::json!({ "jsonrpc": "2.0", "id": 1, "result": result })
                };
                let payload = serde_json::to_vec(&response).unwrap();
                let header = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    payload.len()
                );
                let _ = stream.write_all(header.as_bytes());
                let _ = stream.write_all(&payload);
                let _ = stream.flush();
            }
        });
        format!("http://{addr}/rpc")
    }

    #[test]
    #[cfg(feature = "native-test-runner")]
    fn declared_class_feed_audits_sierra_declarations_and_lists_cairo0_ones() {
        let _env = crate::test_env_shared();
        let root = std::env::temp_dir().join(format!("uc-declared-feed-{}-{}", std::process::id(), epoch_ms_u64().unwrap_or(1)));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(
            root.join("Scarb.toml"),
            "[package]\nname = \"feed_demo\"\nversion = \"0.1.0\"\nedition = \"2024_07\"\n\n[dependencies]\nstarknet = \"2.18.0\"\n\n[[target.starknet-contract]]\nsierra = true\ncasm = false\n",
        )
        .unwrap();
        fs::write(
            root.join("src/lib.cairo"),
            r#"
#[starknet::interface]
pub trait IFee<T> { fn set_fee(ref self: T, fee: u16); fn fee(self: @T) -> u16; }
#[starknet::contract]
pub mod Fee {
    use starknet::storage::{StoragePointerReadAccess, StoragePointerWriteAccess};
    #[storage]
    struct Storage { fee: u16 }
    #[abi(embed_v0)]
    impl FeeImpl of super::IFee<ContractState> {
        fn set_fee(ref self: ContractState, fee: u16) { self.fee.write(fee); }
        fn fee(self: @ContractState) -> u16 { self.fee.read() }
    }
}
"#,
        )
        .unwrap();
        run_build(BuildArgs {
            common: BuildCommonArgs {
                manifest_path: Some(root.join("Scarb.toml")),
                package: None,
                workspace: false,
                features: Vec::new(),
                offline: true,
                release: false,
                profile: Some("dev".to_string()),
            },
            engine: EngineArg::Uc,
            daemon_mode: DaemonModeArg::Off,
            report_path: Some(root.join("build.json")),
            record_failure: None,
            json: false,
            plan_only: false,
        })
        .expect("build");
        let class_text = fs::read_to_string(root.join("target/dev/feed_demo_Fee.contract_class.json")).unwrap();
        let mut class_json: serde_json::Value = serde_json::from_str(&class_text).unwrap();
        // The RPC ships the ABI as a string.
        class_json["abi"] = serde_json::Value::String(serde_json::to_string(&class_json["abi"]).unwrap());
        let rpc = spawn_fake_rpc(class_json, "0xfee");

        let report_path = root.join("feed.json");
        run_declared_class_feed(&AuditArgs {
            rpc_url: Some(rpc),
            declared_since: Some(100),
            declared_until: Some(150),
            max_classes: 10,
            max_depth: 1024,
            max_paths: 1024,
            report_path: Some(report_path.clone()),
            ..AuditArgs::default()
        })
        .expect("feed");
        let report: serde_json::Value = serde_json::from_slice(&fs::read(&report_path).unwrap()).unwrap();
        let _ = fs::remove_dir_all(&root);

        assert_eq!(report["from_block"], 100);
        assert_eq!(report["to_block"], 102, "the range is clamped to the chain head");
        assert_eq!(report["blocks_scanned"], 3);
        assert_eq!(report["classes_declared"], 2);
        assert_eq!(report["classes_audited"], 1);
        let classes = report["classes"].as_array().unwrap();
        let cairo0 = classes.iter().find(|c| c["class_hash"] == "0xdeadc0de").expect("cairo0 entry");
        assert_eq!(cairo0["audited"], false);
        let sierra = classes.iter().find(|c| c["class_hash"] == "0xfee").expect("sierra entry");
        assert_eq!(sierra["audited"], true);
        assert_eq!(sierra["block_number"], 101);
        assert!(sierra["functions"].as_u64().unwrap() >= 2, "{sierra}");
        let findings: Vec<&str> = sierra["findings"].as_array().unwrap().iter().filter_map(|f| f.as_str()).collect();
        assert!(
            findings.iter().any(|f| f.contains("unprotected-state-mutation") && f.contains("set_fee")),
            "the unguarded setter must be flagged: {findings:?}"
        );
        assert_eq!(sierra["grade"], "B");
    }

    #[test]
    fn missing_watch_state_loads_as_empty() {
        let state = load_watch_state(Path::new("/nonexistent/uc-watch/state.json"));
        assert!(state.class_hash.is_none());
        assert!(state.findings.is_empty());
    }
}
