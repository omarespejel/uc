//! SNIP-8 transaction hashes for V3 `INVOKE`, `DECLARE` and `DEPLOY_ACCOUNT`, ECDSA signing and
//! verification. One implementation serves the write commands (which sign) and `uc devnet`
//! (which verifies); the independent check is a real node accepting the transactions.

use super::*;
use starknet_crypto::poseidon_hash_many;
use starknet_types_core::felt::Felt;

const DA_MODE_L1: u64 = 0;
const DA_MODE_L2: u64 = 1;

fn short_string(s: &str) -> Felt {
    Felt::from_bytes_be_slice(s.as_bytes())
}

fn felt_of(v: &serde_json::Value) -> Result<Felt> {
    match v {
        serde_json::Value::String(s) => parse_felt_str(s),
        serde_json::Value::Number(n) => n
            .as_u64()
            .map(Felt::from)
            .ok_or_else(|| anyhow!("number {n} is not a valid felt")),
        other => bail!("expected a felt, got {other}"),
    }
}

fn felts_of(v: Option<&serde_json::Value>) -> Result<Vec<Felt>> {
    v.and_then(|a| a.as_array())
        .map(|a| a.iter().map(felt_of).collect::<Result<Vec<_>>>())
        .unwrap_or_else(|| Ok(Vec::new()))
}

fn da_mode(v: Option<&serde_json::Value>) -> Result<u64> {
    match v.and_then(|m| m.as_str()).unwrap_or("L1") {
        "L1" => Ok(DA_MODE_L1),
        "L2" => Ok(DA_MODE_L2),
        other => bail!("unknown data availability mode `{other}`"),
    }
}

/// `(resource name << 192) | (max_amount << 128) | max_price_per_unit`.
pub(crate) fn resource_bound_felt(name: &str, max_amount: u128, max_price_per_unit: u128) -> Felt {
    let name = Felt::from_bytes_be_slice(name.as_bytes());
    let shifted_name = name * Felt::from(2u128).pow(192u32);
    shifted_name + Felt::from(max_amount) * Felt::from(2u128).pow(128u32) + Felt::from(max_price_per_unit)
}

fn bound_from_json(bounds: &serde_json::Value, key: &str, name: &str) -> Result<Felt> {
    let b = bounds.get(key).cloned().unwrap_or_default();
    let amount = u128_of(b.get("max_amount"))?;
    let price = u128_of(b.get("max_price_per_unit"))?;
    Ok(resource_bound_felt(name, amount, price))
}

fn u128_of(v: Option<&serde_json::Value>) -> Result<u128> {
    let Some(v) = v else { return Ok(0) };
    let f = felt_of(v)?;
    u128::try_from(f.to_biguint()).map_err(|_| anyhow!("value {v} does not fit u128"))
}

/// `poseidon(tip, L1_GAS bound, L2_GAS bound, L1_DATA bound)` as Starknet 0.13.4+ hashes it.
fn fee_fields_hash(tx: &serde_json::Value) -> Result<Felt> {
    let bounds = tx.get("resource_bounds").cloned().unwrap_or_default();
    let tip = felt_of(tx.get("tip").unwrap_or(&serde_json::json!("0x0")))?;
    Ok(poseidon_hash_many(&[
        tip,
        bound_from_json(&bounds, "l1_gas", "L1_GAS")?,
        bound_from_json(&bounds, "l2_gas", "L2_GAS")?,
        bound_from_json(&bounds, "l1_data_gas", "L1_DATA")?,
    ]))
}

/// Version of a V3 query transaction (`2^128 + 3`): nodes run it for estimates and simulations
/// only, so a signature over one can never be replayed as a real transaction.
pub(crate) const QUERY_VERSION_V3: &str = "0x100000000000000000000000000000003";

fn common_prefix(kind: &str, tx: &serde_json::Value, sender_field: &str, chain_id: Felt) -> Result<Vec<Felt>> {
    let version = felt_of(tx.get("version").unwrap_or(&serde_json::json!("0x3")))?;
    if version != Felt::from(3u64) && version != Felt::from_hex(QUERY_VERSION_V3)? {
        bail!("only V3 transactions are hashed (got version {version:#x})");
    }
    let sender = felt_of(tx.get(sender_field).ok_or_else(|| anyhow!("{sender_field} is required"))?)?;
    let nonce = felt_of(tx.get("nonce").unwrap_or(&serde_json::json!("0x0")))?;
    let da = Felt::from(
        (da_mode(tx.get("nonce_data_availability_mode"))? << 32) + da_mode(tx.get("fee_data_availability_mode"))?,
    );
    Ok(vec![
        short_string(kind),
        version,
        sender,
        fee_fields_hash(tx)?,
        poseidon_hash_many(&felts_of(tx.get("paymaster_data"))?),
        chain_id,
        nonce,
        da,
    ])
}

/// The SNIP-8 hash of a V3 transaction given as the JSON the RPC carries.
pub(crate) fn v3_transaction_hash(tx: &serde_json::Value, chain_id: Felt) -> Result<Felt> {
    let kind = tx.get("type").and_then(|t| t.as_str()).unwrap_or("INVOKE");
    match kind {
        "INVOKE" => {
            let mut fields = common_prefix("invoke", tx, "sender_address", chain_id)?;
            fields.push(poseidon_hash_many(&felts_of(tx.get("account_deployment_data"))?));
            fields.push(poseidon_hash_many(&felts_of(tx.get("calldata"))?));
            Ok(poseidon_hash_many(&fields))
        }
        "DECLARE" => {
            let mut fields = common_prefix("declare", tx, "sender_address", chain_id)?;
            fields.push(poseidon_hash_many(&felts_of(tx.get("account_deployment_data"))?));
            fields.push(felt_of(tx.get("class_hash").ok_or_else(|| anyhow!("class_hash is required"))?)?);
            fields.push(felt_of(tx.get("compiled_class_hash").ok_or_else(|| anyhow!("compiled_class_hash is required"))?)?);
            Ok(poseidon_hash_many(&fields))
        }
        "DEPLOY_ACCOUNT" => {
            let class_hash = felt_of(tx.get("class_hash").ok_or_else(|| anyhow!("class_hash is required"))?)?;
            let salt = felt_of(tx.get("contract_address_salt").ok_or_else(|| anyhow!("contract_address_salt is required"))?)?;
            let calldata = felts_of(tx.get("constructor_calldata"))?;
            let address = cairo_lang_runner::casm_run::contract_address::calculate_contract_address(&salt, &class_hash, &calldata, &Felt::ZERO);
            let mut with_address = tx.clone();
            with_address["__address"] = serde_json::json!(format_felt_hex(&address));
            let mut fields = common_prefix("deploy_account", &with_address, "__address", chain_id)?;
            fields.push(poseidon_hash_many(&calldata));
            fields.push(class_hash);
            fields.push(salt);
            Ok(poseidon_hash_many(&fields))
        }
        other => bail!("cannot hash transaction type `{other}`"),
    }
}

/// `(r, s)` ECDSA signature of `hash` with `private_key` (RFC 6979 nonce).
pub(crate) fn sign_transaction_hash(private_key: &Felt, hash: &Felt) -> Result<(Felt, Felt)> {
    use starknet_crypto::{rfc6979_generate_k, sign};
    let k = rfc6979_generate_k(hash, private_key, None);
    let sig = sign(private_key, hash, &k).map_err(|e| anyhow!("ECDSA sign failed: {e}"))?;
    Ok((sig.r, sig.s))
}

/// Whether `(r, s)` is a valid signature of `hash` for `public_key`.
pub(crate) fn verify_transaction_signature(public_key: &Felt, hash: &Felt, r: &Felt, s: &Felt) -> bool {
    starknet_crypto::verify(public_key, hash, r, s).unwrap_or(false)
}

/// Signs `tx` in place: computes its SNIP-8 hash for `chain_id`, sets `signature` to `[r, s]`
/// and returns the hash. With no key the signature is left empty and the hash still returned.
pub(crate) fn sign_v3_transaction(tx: &mut serde_json::Value, chain_id: Felt, private_key: Option<&Felt>) -> Result<Felt> {
    let hash = v3_transaction_hash(tx, chain_id)?;
    if let Some(pk) = private_key {
        let (r, s) = sign_transaction_hash(pk, &hash)?;
        tx["signature"] = serde_json::json!([format_felt_hex(&r), format_felt_hex(&s)]);
    }
    Ok(hash)
}

/// The chain id of the node at `rpc_url`, else the one implied by `--network`, else zero.
pub(crate) fn resolve_chain_id(rpc_url: Option<&str>, network: Option<&str>) -> Felt {
    if let Some(rpc) = rpc_url {
        if let Ok(v) = starknet_rpc_request(rpc, "starknet_chainId", serde_json::json!([])) {
            if let Some(f) = v.as_str().and_then(|s| parse_felt_str(s).ok()) {
                return f;
            }
        }
    }
    match network.map(|n| n.to_ascii_lowercase()).as_deref() {
        Some("mainnet") | Some("alpha-mainnet") => short_string("SN_MAIN"),
        Some("sepolia") | Some("alpha-sepolia") | Some("testnet") | Some("devnet") | Some("katana") => short_string("SN_SEPOLIA"),
        _ => Felt::ZERO,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resource_bound_packs_name_amount_and_price() {
        // L1_GAS = 0x4c315f474153; amount 1, price 2.
        let b = resource_bound_felt("L1_GAS", 1, 2);
        let expected = Felt::from_hex("0x4c315f474153").unwrap() * Felt::from(2u128).pow(192u32)
            + Felt::from(1u64) * Felt::from(2u128).pow(128u32)
            + Felt::from(2u64);
        assert_eq!(b, expected);
        assert_eq!(resource_bound_felt("L2_GAS", 0, 0), Felt::from_hex("0x4c325f474153").unwrap() * Felt::from(2u128).pow(192u32));
    }

    #[test]
    fn hashes_are_deterministic_and_sensitive_to_every_field() {
        let chain = short_string("SN_SEPOLIA");
        let base = serde_json::json!({
            "type": "INVOKE", "version": "0x3", "sender_address": "0x123", "calldata": ["0x1", "0x2"],
            "resource_bounds": { "l1_gas": { "max_amount": "0x10", "max_price_per_unit": "0x20" }, "l2_gas": { "max_amount": "0x0", "max_price_per_unit": "0x0" }, "l1_data_gas": { "max_amount": "0x5", "max_price_per_unit": "0x6" } },
            "tip": "0x0", "paymaster_data": [], "account_deployment_data": [],
            "nonce_data_availability_mode": "L1", "fee_data_availability_mode": "L1", "nonce": "0x7", "signature": []
        });
        let h = v3_transaction_hash(&base, chain).unwrap();
        assert_eq!(h, v3_transaction_hash(&base, chain).unwrap());
        for (field, value) in [
            ("nonce", serde_json::json!("0x8")),
            ("sender_address", serde_json::json!("0x124")),
            ("calldata", serde_json::json!(["0x1"])),
            ("tip", serde_json::json!("0x1")),
            ("nonce_data_availability_mode", serde_json::json!("L2")),
        ] {
            let mut changed = base.clone();
            changed[field] = value;
            assert_ne!(v3_transaction_hash(&changed, chain).unwrap(), h, "{field} must change the hash");
        }
        let mut bounds = base.clone();
        bounds["resource_bounds"]["l2_gas"]["max_amount"] = serde_json::json!("0x1");
        assert_ne!(v3_transaction_hash(&bounds, chain).unwrap(), h);
        assert_ne!(v3_transaction_hash(&base, short_string("SN_MAIN")).unwrap(), h);
        assert!(v3_transaction_hash(&serde_json::json!({ "type": "INVOKE", "version": "0x1", "sender_address": "0x1" }), chain).is_err());
    }

    #[test]
    fn declare_and_deploy_account_hashes_include_their_own_fields() {
        let chain = short_string("SN_SEPOLIA");
        let declare = serde_json::json!({
            "type": "DECLARE", "version": "0x3", "sender_address": "0x123", "class_hash": "0xabc", "compiled_class_hash": "0xdef",
            "resource_bounds": { "l1_gas": { "max_amount": "0x1", "max_price_per_unit": "0x1" }, "l2_gas": { "max_amount": "0x0", "max_price_per_unit": "0x0" }, "l1_data_gas": { "max_amount": "0x0", "max_price_per_unit": "0x0" } },
            "nonce": "0x0"
        });
        let h = v3_transaction_hash(&declare, chain).unwrap();
        let mut other = declare.clone();
        other["compiled_class_hash"] = serde_json::json!("0xdee");
        assert_ne!(v3_transaction_hash(&other, chain).unwrap(), h);

        let deploy = serde_json::json!({
            "type": "DEPLOY_ACCOUNT", "version": "0x3", "class_hash": "0xabc", "contract_address_salt": "0x5", "constructor_calldata": ["0x9"],
            "resource_bounds": { "l1_gas": { "max_amount": "0x1", "max_price_per_unit": "0x1" }, "l2_gas": { "max_amount": "0x0", "max_price_per_unit": "0x0" }, "l1_data_gas": { "max_amount": "0x0", "max_price_per_unit": "0x0" } },
            "nonce": "0x0"
        });
        let d = v3_transaction_hash(&deploy, chain).unwrap();
        let mut other = deploy.clone();
        other["contract_address_salt"] = serde_json::json!("0x6");
        assert_ne!(v3_transaction_hash(&other, chain).unwrap(), d);
    }

    /// Transactions accepted by starknet-devnet 0.10.1 (Starknet RPC 0.10.2, chain SN_SEPOLIA).
    /// The DECLARE was built and signed by sncast 0.60 (starknet-rs), the others by uc; in all
    /// three the node reported `transaction_hash` after recomputing it, so these are
    /// node-attested SNIP-8 vectors.
    #[test]
    fn v3_hashes_match_node_attested_vectors() {
        let chain = short_string("SN_SEPOLIA");
        let vectors = [
            r#"{"transaction_hash": "0x263d232968eb61f080469761ff1639679e595c68baf4d39ca1a45154df7973", "type": "DECLARE", "version": "0x3", "nonce": "0x0", "resource_bounds": {"l1_gas": {"max_amount": "0x0", "max_price_per_unit": "0x59682f00"}, "l1_data_gas": {"max_amount": "0x120", "max_price_per_unit": "0x59682f00"}, "l2_gas": {"max_amount": "0x34f49a0", "max_price_per_unit": "0x59682f00"}}, "tip": "0x0", "paymaster_data": [], "nonce_data_availability_mode": "L1", "fee_data_availability_mode": "L1", "sender_address": "0x260a8311b4f1092db620b923e8d7d20e76dedcc615fb4b6fdf28315b81de201", "compiled_class_hash": "0x24c9bb910638f458bc22d0ac057d438fa6ec0ea4d0f531c2c037368c61d07ac", "class_hash": "0x2e895de4b82503229f12daf608c7f99ac1b9933d5ce77ff57fa7832453f2391", "account_deployment_data": []}"#,
            r#"{"transaction_hash": "0x5d7a1c422e8a1959a39da772a53b7133611fc2b64e2d7d179da11c04c696dfb", "type": "INVOKE", "version": "0x3", "nonce": "0x2", "resource_bounds": {"l1_gas": {"max_amount": "0x1", "max_price_per_unit": "0x59682f00"}, "l1_data_gas": {"max_amount": "0x180", "max_price_per_unit": "0x59682f00"}, "l2_gas": {"max_amount": "0x161ca0", "max_price_per_unit": "0x59682f00"}}, "tip": "0x0", "paymaster_data": [], "nonce_data_availability_mode": "L1", "fee_data_availability_mode": "L1", "account_deployment_data": [], "sender_address": "0x260a8311b4f1092db620b923e8d7d20e76dedcc615fb4b6fdf28315b81de201", "calldata": ["0x1", "0x4a7c8d27155fdd205908f671a63355564b9ff05b504a1c3d81f1b713ffdd442", "0x9be09a9a955d79baeb70c3b225ab5090fb9bc543cf23547b9f095c38a27574", "0x1", "0x2"]}"#,
            r#"{"transaction_hash": "0x2791d27afaee3756bc9be9d0c6cdb98585ada7e00f046d74edda2546e50fec4", "type": "DEPLOY_ACCOUNT", "version": "0x3", "nonce": "0x0", "resource_bounds": {"l1_gas": {"max_amount": "0x1", "max_price_per_unit": "0x59682f00"}, "l1_data_gas": {"max_amount": "0x2a0", "max_price_per_unit": "0x59682f00"}, "l2_gas": {"max_amount": "0x313f08", "max_price_per_unit": "0x59682f00"}}, "tip": "0x0", "paymaster_data": [], "nonce_data_availability_mode": "L1", "fee_data_availability_mode": "L1", "contract_address_salt": "0x42", "constructor_calldata": ["0x40e67ff2c1b2b9fd7cf483125ac0c1647f4165cc0b8f860adb8b489e90d0468"], "class_hash": "0x5b4b537eaa2399e3aa99c4e2e0208ebd6c71bc1467938cd52c798c601e43564"}"#,
        ];
        for raw in vectors {
            let tx: serde_json::Value = serde_json::from_str(raw).unwrap();
            let expected = parse_felt_str(tx["transaction_hash"].as_str().unwrap()).unwrap();
            assert_eq!(v3_transaction_hash(&tx, chain).unwrap(), expected, "{} hash", tx["type"]);
        }
    }

    #[test]
    fn signing_round_trips_through_verification() {
        let pk = Felt::from_hex("0x1234abcd").unwrap();
        let pubkey = starknet_crypto::get_public_key(&pk);
        let mut tx = serde_json::json!({ "type": "INVOKE", "version": "0x3", "sender_address": "0x1", "calldata": [], "nonce": "0x0" });
        let hash = sign_v3_transaction(&mut tx, short_string("SN_SEPOLIA"), Some(&pk)).unwrap();
        let sig = tx["signature"].as_array().unwrap();
        let r = parse_felt_str(sig[0].as_str().unwrap()).unwrap();
        let s = parse_felt_str(sig[1].as_str().unwrap()).unwrap();
        assert!(verify_transaction_signature(&pubkey, &hash, &r, &s));
        assert!(!verify_transaction_signature(&pubkey, &(hash + Felt::ONE), &r, &s));
        assert!(!verify_transaction_signature(&starknet_crypto::get_public_key(&Felt::from(7u64)), &hash, &r, &s));
    }
}
