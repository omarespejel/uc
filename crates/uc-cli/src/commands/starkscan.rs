use super::*;
use std::collections::HashMap;

/// Starkscan's public API: `https://starkscan.co/api/v1/{chain}/...`, authenticated with the
/// `X-Starkscan-Api-Key` header (`STARKSCAN_API_KEY`). The JSON-RPC gateway lives at
/// `/v1/{chain}/rpc`; a URL-token form `https://starkscan.co/rpc/v0_10/{chain}/<key>` exists
/// for tools that cannot set headers.
pub(crate) const STARKSCAN_API_BASE: &str = "https://starkscan.co/api";
pub(crate) const STARKSCAN_API_KEY_HEADER: &str = "X-Starkscan-Api-Key";
pub(crate) const STARKSCAN_API_KEY_ENV: &str = "STARKSCAN_API_KEY";

pub(crate) fn starkscan_api_key() -> Option<String> {
    std::env::var(STARKSCAN_API_KEY_ENV)
        .ok()
        .map(|k| k.trim().to_string())
        .filter(|k| !k.is_empty())
}

/// Starkscan chain id for a `--network` preset (`mainnet` / `sepolia` and their aliases).
pub(crate) fn starkscan_chain_id(network: Option<&str>) -> Option<&'static str> {
    match network.map(|n| n.trim().to_ascii_lowercase()).as_deref() {
        Some("mainnet") | Some("alpha-mainnet") | Some("sn_main") | None => Some("SN_MAIN"),
        Some("sepolia") | Some("alpha-sepolia") | Some("testnet") | Some("sn_sepolia") => Some("SN_SEPOLIA"),
        _ => None,
    }
}

/// Chain id of a URL that points at Starkscan (`.../v1/SN_MAIN/...` or `.../rpc/v0_10/SN_MAIN/...`).
pub(crate) fn starkscan_chain_from_url(url: &str) -> Option<&'static str> {
    if !is_starkscan_url(url) {
        return None;
    }
    if url.contains("/SN_SEPOLIA") {
        Some("SN_SEPOLIA")
    } else if url.contains("/SN_MAIN") {
        Some("SN_MAIN")
    } else {
        None
    }
}

pub(crate) fn is_starkscan_url(url: &str) -> bool {
    url.starts_with("https://starkscan.co/") || url.starts_with("https://www.starkscan.co/")
}

/// Header-authenticated JSON-RPC gateway URL for `chain`.
pub(crate) fn starkscan_rpc_url(chain: &str) -> String {
    format!("{STARKSCAN_API_BASE}/v1/{chain}/rpc")
}

/// Request headers for a Starkscan URL: the API key when configured.
pub(crate) fn starkscan_headers(url: &str) -> Vec<(String, String)> {
    if !is_starkscan_url(url) {
        return Vec::new();
    }
    starkscan_api_key()
        .map(|k| vec![(STARKSCAN_API_KEY_HEADER.to_string(), k)])
        .unwrap_or_default()
}

/// A JSON request against Starkscan's API. `None` for a 404 (no such resource).
fn starkscan_request(method: &str, path: &str, body: Option<&serde_json::Value>) -> Result<Option<serde_json::Value>> {
    let url = format!("{STARKSCAN_API_BASE}{path}");
    let key = starkscan_api_key()
        .ok_or_else(|| anyhow!("Starkscan API needs {STARKSCAN_API_KEY_ENV} (header {STARKSCAN_API_KEY_HEADER})"))?;
    let headers = [(STARKSCAN_API_KEY_HEADER, key.as_str())];
    let resp = http_request_json(method, &url, body, &headers)?;
    if resp.status == 404 {
        return Ok(None);
    }
    if resp.status >= 400 {
        bail!("Starkscan API {method} {path} failed with HTTP {}: {}", resp.status, resp.body);
    }
    Ok(Some(resp.body))
}

/// `GET /v1/{chain}/contract/{address}/verification`: how Starkscan sees the contract's source
/// verification (`verificationStatus` pending/verified/failed, `sourceRef`, `compilerVersion`).
pub(crate) fn starkscan_contract_verification(chain: &str, address: &str) -> Result<Option<serde_json::Value>> {
    starkscan_request("GET", &format!("/v1/{chain}/contract/{address}/verification"), None)
}

/// `PUT /v1/{chain}/contract/{address}/verification` (needs a key with the verification scope).
pub(crate) fn starkscan_put_contract_verification(
    chain: &str,
    address: &str,
    body: &serde_json::Value,
) -> Result<serde_json::Value> {
    starkscan_request("PUT", &format!("/v1/{chain}/contract/{address}/verification"), Some(body))?
        .ok_or_else(|| anyhow!("Starkscan verification route not found for {address} on {chain}"))
}

/// What Starkscan knows about an address: its alias (label), whether it is an account or a
/// token, and the token symbol and name when it is one.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct StarkscanLabel {
    pub address: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alias: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_account: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_token: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token_kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token_symbol: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub class_hash: Option<String>,
}

impl StarkscanLabel {
    /// One short label for display: alias, else token symbol, else nothing.
    pub(crate) fn display(&self) -> Option<String> {
        self.alias
            .clone()
            .or_else(|| self.token_symbol.clone())
            .or_else(|| self.is_account.filter(|a| *a).map(|_| "account".to_string()))
    }
}

/// Cached label lookups for one chain; every address is fetched at most once per run.
pub(crate) struct StarkscanLabels {
    chain: String,
    cache: HashMap<String, StarkscanLabel>,
    pub(crate) requests: usize,
}

impl StarkscanLabels {
    pub(crate) fn new(chain: &str) -> Self {
        Self { chain: chain.to_string(), cache: HashMap::new(), requests: 0 }
    }

    pub(crate) fn lookup(&mut self, address: &str) -> StarkscanLabel {
        let key = parse_felt_str(address).map(|f| format_felt_hex(&f)).unwrap_or_else(|_| address.to_string());
        if let Some(l) = self.cache.get(&key) {
            return l.clone();
        }
        let mut label = StarkscanLabel { address: key.clone(), ..Default::default() };
        if starkscan_api_key().is_none() {
            return label;
        }
        self.requests += 1;
        if let Ok(Some(meta)) = starkscan_request("GET", &format!("/v1/{}/contract/{key}", self.chain), None) {
            label.alias = meta.get("alias").and_then(|v| v.as_str()).map(str::to_string);
            label.is_account = meta.get("isAccount").and_then(|v| v.as_bool());
            label.is_token = meta.get("isToken").and_then(|v| v.as_bool());
            label.token_kind = meta.get("tokenKind").and_then(|v| v.as_str()).map(str::to_string);
            label.class_hash = meta.get("classHash").and_then(|v| v.as_str()).map(str::to_string);
            if label.is_token == Some(true) {
                self.requests += 1;
                if let Ok(Some(token)) = starkscan_request("GET", &format!("/v1/{}/token/{key}", self.chain), None) {
                    label.token_symbol = token.get("symbol").and_then(|v| v.as_str()).map(str::to_string);
                    label.token_name = token.get("name").and_then(|v| v.as_str()).map(str::to_string);
                }
            }
            self.cache.insert(key, label.clone());
        }
        label
    }
}

/// `{selector hex: function name}` for every function, constructor and L1 handler an ABI
/// declares (top level and inside interfaces).
pub(crate) fn abi_selector_names(abi: &serde_json::Value) -> HashMap<String, String> {
    use cairo_lang_starknet_classes::keccak::starknet_keccak;
    let mut out = HashMap::new();
    fn visit(items: &serde_json::Value, out: &mut HashMap<String, String>) {
        let Some(arr) = items.as_array() else { return };
        for item in arr {
            match item.get("type").and_then(|t| t.as_str()) {
                Some("function") | Some("constructor") | Some("l1_handler") => {
                    if let Some(name) = item.get("name").and_then(|n| n.as_str()) {
                        let selector = format!("{:#x}", starknet_keccak(name.as_bytes()));
                        out.insert(selector, name.to_string());
                    }
                }
                Some("interface") => {
                    if let Some(inner) = item.get("items") {
                        visit(inner, out);
                    }
                }
                _ => {}
            }
        }
    }
    visit(abi, &mut out);
    out
}

/// Resolves selectors to function names from the classes a trace touches: one
/// `starknet_getClass` per distinct class hash.
pub(crate) struct ClassSelectorNames {
    rpc: String,
    by_class: HashMap<String, HashMap<String, String>>,
    pub(crate) class_fetches: usize,
}

impl ClassSelectorNames {
    pub(crate) fn new(rpc: &str) -> Self {
        Self { rpc: rpc.to_string(), by_class: HashMap::new(), class_fetches: 0 }
    }

    pub(crate) fn name(&mut self, class_hash: Option<&str>, selector: &str) -> Option<String> {
        let class_hash = class_hash?;
        if !self.by_class.contains_key(class_hash) {
            self.class_fetches += 1;
            let names = starknet_rpc_request(
                &self.rpc,
                "starknet_getClass",
                serde_json::json!({ "block_id": "latest", "class_hash": class_hash }),
            )
            .ok()
            .and_then(|class| {
                let abi = class.get("abi")?.clone();
                let abi = match abi {
                    serde_json::Value::String(text) => serde_json::from_str(&text).ok()?,
                    other => other,
                };
                Some(abi_selector_names(&abi))
            })
            .unwrap_or_default();
            self.by_class.insert(class_hash.to_string(), names);
        }
        let wanted = parse_felt_str(selector).map(|f| format_felt_hex(&f)).unwrap_or_else(|_| selector.to_string());
        self.by_class.get(class_hash)?.get(&wanted).cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chain_ids_and_rpc_url_follow_starkscan_conventions() {
        assert_eq!(starkscan_chain_id(Some("mainnet")), Some("SN_MAIN"));
        assert_eq!(starkscan_chain_id(Some("alpha-sepolia")), Some("SN_SEPOLIA"));
        assert_eq!(starkscan_chain_id(Some("devnet")), None);
        assert_eq!(starkscan_rpc_url("SN_MAIN"), "https://starkscan.co/api/v1/SN_MAIN/rpc");
        assert_eq!(starkscan_chain_from_url("https://starkscan.co/rpc/v0_10/SN_SEPOLIA/abc"), Some("SN_SEPOLIA"));
        assert_eq!(starkscan_chain_from_url("https://api.cartridge.gg/x/starknet/mainnet"), None);
        assert!(starkscan_headers("https://api.cartridge.gg/x/starknet/mainnet").is_empty());
    }

    #[test]
    fn abi_selector_names_cover_interfaces_constructors_and_handlers() {
        use cairo_lang_starknet_classes::keccak::starknet_keccak;
        let abi = serde_json::json!([
            {"type": "constructor", "name": "constructor", "inputs": []},
            {"type": "l1_handler", "name": "handle_deposit", "inputs": []},
            {"type": "interface", "name": "IERC20", "items": [
                {"type": "function", "name": "transfer", "inputs": [], "outputs": [], "state_mutability": "external"}
            ]},
            {"type": "event", "name": "Transfer"},
            {"type": "function", "name": "upgrade", "inputs": []}
        ]);
        let names = abi_selector_names(&abi);
        let sel = |n: &str| format!("{:#x}", starknet_keccak(n.as_bytes()));
        assert_eq!(names.get(&sel("transfer")).map(String::as_str), Some("transfer"));
        assert_eq!(names.get(&sel("upgrade")).map(String::as_str), Some("upgrade"));
        assert_eq!(names.get(&sel("constructor")).map(String::as_str), Some("constructor"));
        assert_eq!(names.get(&sel("handle_deposit")).map(String::as_str), Some("handle_deposit"));
        assert_eq!(names.len(), 4, "events are not selectors");
    }

    #[test]
    fn label_display_prefers_alias_then_symbol_then_account() {
        let mut l = StarkscanLabel { address: "0x1".into(), ..Default::default() };
        assert_eq!(l.display(), None);
        l.is_account = Some(true);
        assert_eq!(l.display().as_deref(), Some("account"));
        l.token_symbol = Some("STRK".into());
        assert_eq!(l.display().as_deref(), Some("STRK"));
        l.alias = Some("Starknet Token".into());
        assert_eq!(l.display().as_deref(), Some("Starknet Token"));
    }
}
