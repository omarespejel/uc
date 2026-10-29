use super::*;

/// Converts a parsed `Scarb.toml` (and optional `snfoundry.toml`) into a clean next-generation
/// `uc.toml` document (`[project]`, implicit `starknet`/`snforge_std`, unified `[prove]`, `[test]`,
/// and `[networks.<name>]`).
#[allow(dead_code)]
pub(crate) fn convert_scarb_and_snfoundry_to_uc_toml(
    scarb_doc: &TomlValue,
    snfoundry_doc: Option<&TomlValue>,
) -> Result<String> {
    convert_scarb_and_snfoundry_to_uc_toml_with_workspace(scarb_doc, snfoundry_doc, None)
}

pub(crate) fn convert_scarb_and_snfoundry_to_uc_toml_with_workspace(
    scarb_doc: &TomlValue,
    snfoundry_doc: Option<&TomlValue>,
    workspace_doc: Option<&TomlValue>,
) -> Result<String> {
    let mut root = toml::map::Map::new();
    let ws_pkg_table = scarb_doc
        .get("workspace")
        .and_then(|w| w.get("package").or_else(|| w.get("project")))
        .and_then(TomlValue::as_table)
        .or_else(|| {
            workspace_doc
                .and_then(|d| d.get("workspace"))
                .and_then(|w| w.get("package").or_else(|| w.get("project")))
                .and_then(TomlValue::as_table)
        });

    if let Some(pkg) = scarb_doc.get("package").and_then(TomlValue::as_table) {
        let mut proj = toml::map::Map::new();
        for (k, v) in pkg {
            let inherits_ws = v
                .as_table()
                .and_then(|t| t.get("workspace"))
                .and_then(TomlValue::as_bool)
                == Some(true);
            if inherits_ws {
                if let Some(resolved) = ws_pkg_table.and_then(|wp| wp.get(k)) {
                    proj.insert(k.clone(), resolved.clone());
                } else if k == "version" {
                    proj.insert(k.clone(), TomlValue::String("0.1.0".to_string()));
                } else if k == "edition" {
                    proj.insert(k.clone(), TomlValue::String("2024_07".to_string()));
                } else {
                    proj.insert(k.clone(), v.clone());
                }
            } else {
                proj.insert(k.clone(), v.clone());
            }
        }
        proj.entry("version".to_string())
            .or_insert_with(|| TomlValue::String("0.1.0".to_string()));
        proj.entry("edition".to_string())
            .or_insert_with(|| TomlValue::String("2024_07".to_string()));
        root.insert("project".to_string(), TomlValue::Table(proj));
    } else if let Some(proj) = scarb_doc.get("project").and_then(TomlValue::as_table) {
        root.insert("project".to_string(), TomlValue::Table(proj.clone()));
    }

    if let Some(ws) = scarb_doc.get("workspace").and_then(TomlValue::as_table) {
        let mut clean_ws = ws.clone();
        if let Some(ws_deps) = clean_ws
            .get_mut("dependencies")
            .and_then(TomlValue::as_table_mut)
        {
            for builtin in [
                "starknet",
                "snforge_std",
                "sncast_std",
                "assert_macros",
                "cairo_test",
                "cairo_execute",
                "cairo_run",
                "snforge_scarb_plugin",
            ] {
                ws_deps.remove(builtin);
            }
        }
        root.insert("workspace".to_string(), TomlValue::Table(clean_ws));
    }

    for (section_keys, out_section) in [
        (&["dependencies"][..], "dependencies"),
        (&["dev-dependencies", "dev_dependencies"][..], "dev-dependencies"),
    ] {
        let mut saw_section = false;
        let mut clean_deps = toml::map::Map::new();
        for key in section_keys {
            if let Some(deps) = scarb_doc.get(*key).and_then(TomlValue::as_table) {
                saw_section = true;
                for (k, v) in deps {
                    if matches!(
                        k.as_str(),
                        "starknet"
                            | "snforge_std"
                            | "sncast_std"
                            | "assert_macros"
                            | "cairo_test"
                            | "cairo_execute"
                            | "cairo_run"
                            | "snforge_scarb_plugin"
                    ) {
                        continue;
                    }
                    clean_deps.insert(k.clone(), v.clone());
                }
            }
        }
        if saw_section && (!clean_deps.is_empty() || out_section == "dependencies") {
            root.insert(out_section.to_string(), TomlValue::Table(clean_deps));
        }
    }

    for passthrough in [
        "features",
        "cairo",
        "profile",
        "patch",
        "target",
        "lib",
        "executable",
        "cairo-plugin",
        "scripts",
    ] {
        if let Some(val) = scarb_doc.get(passthrough) {
            root.insert(passthrough.to_string(), val.clone());
        }
    }
    if let Some(tool_tbl) = scarb_doc.get("tool").and_then(TomlValue::as_table) {
        let mut clean_tool = tool_tbl.clone();
        clean_tool.remove("snforge");
        clean_tool.remove("sncast");
        if !clean_tool.is_empty() {
            root.insert("tool".to_string(), TomlValue::Table(clean_tool));
        }
    }

    let mut prove_tbl = scarb_doc
        .get("prove")
        .or_else(|| scarb_doc.get("uc").and_then(|u| u.get("prove")))
        .and_then(TomlValue::as_table)
        .cloned()
        .unwrap_or_default();
    prove_tbl
        .entry("fail-on".to_string())
        .or_insert_with(|| TomlValue::String("counterexample".to_string()));
    root.insert("prove".to_string(), TomlValue::Table(prove_tbl));

    let mut test_tbl = scarb_doc
        .get("test")
        .or_else(|| scarb_doc.get("uc").and_then(|u| u.get("test")))
        .and_then(TomlValue::as_table)
        .cloned()
        .unwrap_or_default();
    if let Some(test_arr) = scarb_doc.get("test").and_then(TomlValue::as_array) {
        let mut ext_contracts = Vec::new();
        for item in test_arr {
            if let Some(arr) = item
                .as_table()
                .and_then(|t| t.get("build-external-contracts"))
                .and_then(TomlValue::as_array)
            {
                for c in arr {
                    if !ext_contracts.contains(c) {
                        ext_contracts.push(c.clone());
                    }
                }
            }
        }
        if !ext_contracts.is_empty() {
            test_tbl
                .entry("build-external-contracts".to_string())
                .or_insert(TomlValue::Array(ext_contracts));
        }
    }
    for snforge_source in [
        scarb_doc.get("tool").and_then(|t| t.get("snforge")),
        snfoundry_doc.and_then(|s| s.get("snforge")),
    ]
    .into_iter()
    .flatten()
    {
        if let Some(snforge_tbl) = snforge_source
            .as_table()
            .and_then(|t| {
                t.get("default")
                    .and_then(TomlValue::as_table)
                    .or_else(|| Some(t))
            })
        {
            if let Some(runs) = snforge_tbl.get("fuzzer_runs").or_else(|| snforge_tbl.get("fuzzer-runs")) {
                test_tbl.insert("fuzzer-runs".to_string(), runs.clone());
            }
            if let Some(seed) = snforge_tbl.get("fuzzer_seed").or_else(|| snforge_tbl.get("fuzzer-seed")) {
                test_tbl.insert("fuzzer-seed".to_string(), seed.clone());
            }
        }
    }
    test_tbl
        .entry("fuzzer-runs".to_string())
        .or_insert_with(|| TomlValue::Integer(256));
    root.insert("test".to_string(), TomlValue::Table(test_tbl));

    // Migrate `[sncast.<profile>]` from `snfoundry.toml` or `[tool.sncast]` from `Scarb.toml` into `[networks.<profile>]`.
    let mut networks_tbl = scarb_doc
        .get("networks")
        .and_then(TomlValue::as_table)
        .cloned()
        .unwrap_or_default();
    for sncast_source in [
        scarb_doc.get("tool").and_then(|t| t.get("sncast")),
        snfoundry_doc.and_then(|s| s.get("sncast")),
    ]
    .into_iter()
    .flatten()
    {
        if let Some(sncast_tbl) = sncast_source.as_table() {
            if sncast_tbl.contains_key("url") || sncast_tbl.contains_key("account") {
                let mut clean_prof = sncast_tbl.clone();
                if let Some(url_val) = clean_prof.remove("url") {
                    clean_prof.entry("rpc".to_string()).or_insert(url_val);
                }
                networks_tbl.insert("default".to_string(), TomlValue::Table(clean_prof));
            } else {
                for (prof_name, prof_val) in sncast_tbl {
                    if let Some(prof_map) = prof_val.as_table() {
                        let mut clean_prof = prof_map.clone();
                        if let Some(url_val) = clean_prof.remove("url") {
                            clean_prof.entry("rpc".to_string()).or_insert(url_val);
                        }
                        networks_tbl.insert(prof_name.clone(), TomlValue::Table(clean_prof));
                    }
                }
            }
        }
    }
    if !networks_tbl.is_empty() {
        root.insert("networks".to_string(), TomlValue::Table(networks_tbl));
    }

    let mut rendered = toml::to_string_pretty(&TomlValue::Table(root))
        .context("failed to encode migrated uc.toml")?;
    if !rendered.ends_with('\n') {
        rendered.push('\n');
    }
    Ok(rendered)
}

pub(crate) fn run_migrate(args: MigrateArgs) -> Result<()> {
    let manifest_path = resolve_manifest_path(&args.manifest_path)?;
    let workspace_root = manifest_path
        .parent()
        .context("manifest path has no parent")?
        .to_path_buf();
    let raw = read_text_file_with_limit(&manifest_path, MAX_MANIFEST_BYTES, "manifest")?;
    let parsed: TomlValue = raw
        .parse()
        .with_context(|| format!("failed to parse TOML in {}", manifest_path.display()))?;

    let ancestor_ws_doc: Option<TomlValue> = workspace_root
        .ancestors()
        .skip(1)
        .take(4)
        .find_map(|anc| {
            for f in ["uc.toml", "Scarb.toml"] {
                let p = anc.join(f);
                if p.is_file() {
                    if let Ok(txt) = fs::read_to_string(&p) {
                        if let Ok(doc) = txt.parse::<TomlValue>() {
                            if doc.get("workspace").is_some() {
                                return Some(doc);
                            }
                        }
                    }
                }
            }
            None
        });
    let ws_pkg_fallback = parsed
        .get("workspace")
        .or_else(|| ancestor_ws_doc.as_ref().and_then(|d| d.get("workspace")))
        .and_then(|w| w.get("package").or_else(|| w.get("project")))
        .and_then(TomlValue::as_table);

    let package = parsed
        .get("package")
        .or_else(|| parsed.get("project"))
        .and_then(TomlValue::as_table);
    let package_name = package
        .and_then(|tbl| tbl.get("name"))
        .and_then(TomlValue::as_str)
        .map(str::to_string);
    let package_version = package
        .and_then(|tbl| tbl.get("version"))
        .and_then(|v| {
            v.as_str().or_else(|| {
                if v.as_table()
                    .and_then(|t| t.get("workspace"))
                    .and_then(TomlValue::as_bool)
                    == Some(true)
                {
                    ws_pkg_fallback
                        .and_then(|wp| wp.get("version"))
                        .and_then(TomlValue::as_str)
                } else {
                    None
                }
            })
        })
        .map(str::to_string);
    let edition = package
        .and_then(|tbl| tbl.get("edition"))
        .and_then(|v| {
            v.as_str().or_else(|| {
                if v.as_table()
                    .and_then(|t| t.get("workspace"))
                    .and_then(TomlValue::as_bool)
                    == Some(true)
                {
                    ws_pkg_fallback
                        .and_then(|wp| wp.get("edition"))
                        .and_then(TomlValue::as_str)
                        .or(Some("2024_07"))
                } else {
                    None
                }
            })
        })
        .map(str::to_string);

    let dependency_count = parsed
        .get("dependencies")
        .and_then(TomlValue::as_table)
        .map_or(0, |tbl| tbl.len());
    let dev_dependency_count = parsed
        .get("dev-dependencies")
        .and_then(TomlValue::as_table)
        .map_or(0, |tbl| tbl.len())
        + parsed
            .get("dev_dependencies")
            .and_then(TomlValue::as_table)
            .map_or(0, |tbl| tbl.len());

    let known_sections = [
        "package",
        "project",
        "dependencies",
        "dev-dependencies",
        "dev_dependencies",
        "workspace",
        "target",
        "scripts",
        "tool",
        "features",
        "patch",
        "cairo",
        "cairo-plugin",
        "lib",
        "executable",
        "test",
        "prove",
        "networks",
        "devnet",
        "profile",
        "uc",
    ];

    let unknown_sections = parsed
        .as_table()
        .map(|tbl| {
            let mut keys: Vec<String> = tbl
                .keys()
                .filter(|k| !known_sections.contains(&k.as_str()))
                .cloned()
                .collect();
            keys.sort();
            keys
        })
        .unwrap_or_default();

    let mut warnings = Vec::new();
    if package_name.is_none() && parsed.get("workspace").is_none() {
        warnings.push("missing [package].name".to_string());
    }
    if edition.is_none() && parsed.get("workspace").is_none() {
        warnings.push("missing [package].edition".to_string());
    }
    if !unknown_sections.is_empty() {
        warnings.push(format!(
            "unknown top-level sections detected: {}",
            unknown_sections.join(", ")
        ));
    }

    let snfoundry_path = workspace_root.join("snfoundry.toml");
    let snfoundry_doc = if snfoundry_path.is_file() {
        fs::read_to_string(&snfoundry_path)
            .ok()
            .and_then(|t| toml::from_str::<TomlValue>(&t).ok())
    } else {
        None
    };

    let target_uc_toml = args
        .emit_uc_toml
        .clone()
        .unwrap_or_else(|| workspace_root.join("uc.toml"));
    let migrated_toml = convert_scarb_and_snfoundry_to_uc_toml_with_workspace(
        &parsed,
        snfoundry_doc.as_ref(),
        ancestor_ws_doc.as_ref(),
    )?;

    if !args.dry_run {
        if let Some(parent) = target_uc_toml.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&target_uc_toml, &migrated_toml)
            .with_context(|| format!("failed to write {}", target_uc_toml.display()))?;

        // Also migrate `Scarb.lock` -> `uc.lock` if present
        let target_dir = target_uc_toml.parent().unwrap_or(&workspace_root);
        let scarb_lock = workspace_root.join("Scarb.lock");
        let uc_lock = target_dir.join("uc.lock");
        if scarb_lock.is_file() && !uc_lock.is_file() {
            if let Ok(lock_text) = fs::read_to_string(&scarb_lock) {
                let converted_lock = lock_text.replace(
                    "# Code generated by scarb DO NOT EDIT.",
                    "# Code generated by uc DO NOT EDIT.",
                );
                let _ = fs::write(&uc_lock, converted_lock);
            }
        }

        // If this is a multi-package workspace, also migrate member `Scarb.toml` files to `uc.toml`
        // when migrating in-place (`--emit-uc-toml` not set).
        if parsed.get("workspace").is_some() && args.emit_uc_toml.is_none() {
            if let Ok(members) = resolve_all_workspace_member_manifests(&manifest_path, &workspace_root) {
                for member_manifest in members {
                    if member_manifest == manifest_path {
                        continue;
                    }
                    if let Some(member_dir) = member_manifest.parent() {
                        let member_uc = member_dir.join("uc.toml");
                        if !member_uc.is_file() {
                            if let Ok(m_raw) = fs::read_to_string(&member_manifest) {
                                if let Ok(m_doc) = m_raw.parse::<TomlValue>() {
                                    if let Ok(m_out) =
                                        convert_scarb_and_snfoundry_to_uc_toml_with_workspace(
                                            &m_doc,
                                            None,
                                            Some(&parsed),
                                        )
                                    {
                                        let _ = fs::write(&member_uc, m_out);
                                        if args.remove_legacy && member_manifest.file_name().and_then(|s| s.to_str()) == Some("Scarb.toml") {
                                            let _ = fs::remove_file(&member_manifest);
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        if args.remove_legacy {
            if manifest_path.file_name().and_then(|s| s.to_str()) == Some("Scarb.toml") {
                let _ = fs::remove_file(&manifest_path);
            }
            let _ = fs::remove_file(workspace_root.join("Scarb.lock"));
            let _ = fs::remove_file(&snfoundry_path);
        }
    }

    let report = MigrationReport {
        generated_at_epoch_ms: epoch_ms()?,
        manifest_path: target_uc_toml.display().to_string(),
        workspace_root: workspace_root.display().to_string(),
        package_name,
        package_version,
        edition,
        dependency_count,
        dev_dependency_count,
        unknown_sections,
        warnings,
        suggested_next_steps: vec![
            "Run `uc build`, `uc test`, and `uc prove` against the generated `uc.toml`.".to_string(),
            "Pass `--remove-legacy` to delete `Scarb.toml`, `Scarb.lock`, and `snfoundry.toml` once verified.".to_string(),
        ],
    };

    if let Some(report_path) = &args.report_path {
        write_json_report(report_path, &report)?;
    }
    if args.json {
        json_println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        println!(
            "uc migrate: {} {}",
            if args.dry_run { "previewed" } else { "wrote" },
            target_uc_toml.display()
        );
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrate_converts_scarb_and_snfoundry_to_uc_toml_and_removes_legacy() {
        let dir = std::env::temp_dir().join(format!(
            "uc-migrate-test-{}-{}",
            std::process::id(),
            epoch_ms_u64().unwrap_or(1)
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("src")).unwrap();
        let scarb_toml = dir.join("Scarb.toml");
        let snfoundry_toml = dir.join("snfoundry.toml");
        let scarb_lock = dir.join("Scarb.lock");
        fs::write(
            &scarb_toml,
            r#"[package]
name = "legacy_pkg"
version = "0.2.0"
edition = "2024_07"

[dependencies]
starknet = "2.11.4"
openzeppelin = "2.0.0"

[dev-dependencies]
snforge_std = "0.39.0"

[[target.starknet-contract]]
sierra = true
casm = true
"#,
        )
        .unwrap();
        fs::write(
            &snfoundry_toml,
            r#"[sncast.sepolia]
url = "https://starknet-sepolia.public.blastapi.io/rpc/v0_8"
account = "deployer"
"#,
        )
        .unwrap();
        fs::write(&scarb_lock, "version = 1\n").unwrap();

        run_migrate(MigrateArgs {
            manifest_path: Some(scarb_toml.clone()),
            emit_uc_toml: None,
            dry_run: false,
            remove_legacy: true,
            report_path: None,
            json: false,
        })
        .expect("uc migrate should succeed");

        assert!(!scarb_toml.exists(), "Scarb.toml should be removed with --remove-legacy");
        assert!(!snfoundry_toml.exists(), "snfoundry.toml should be removed with --remove-legacy");
        assert!(!scarb_lock.exists(), "Scarb.lock should be removed with --remove-legacy");
        assert!(dir.join("uc.lock").is_file(), "uc.lock should be created");
        let uc_raw = fs::read_to_string(dir.join("uc.toml")).unwrap();
        let uc_doc: TomlValue = uc_raw.parse().unwrap();
        assert_eq!(uc_doc["project"]["name"].as_str(), Some("legacy_pkg"));
        assert!(
            uc_doc.get("dependencies").and_then(|d| d.get("starknet")).is_none(),
            "implicit starknet dependency should be stripped: {uc_raw}"
        );
        assert_eq!(
            uc_doc["dependencies"]["openzeppelin"].as_str(),
            Some("2.0.0"),
            "explicit registry dependency preserved: {uc_raw}"
        );
        assert_eq!(
            uc_doc["networks"]["sepolia"]["rpc"].as_str(),
            Some("https://starknet-sepolia.public.blastapi.io/rpc/v0_8"),
            "snfoundry.toml profile migrated to [networks.sepolia]: {uc_raw}"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn migrate_multi_member_workspace_end_to_end_check_build_test_prove_tree() {
        let dir = std::env::temp_dir().join(format!(
            "uc-migrate-ws-e2e-{}-{}",
            std::process::id(),
            epoch_ms_u64().unwrap_or(2)
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("packages/macros")).unwrap();
        fs::create_dir_all(dir.join("packages/math_lib/src")).unwrap();
        fs::create_dir_all(dir.join("packages/vault/src")).unwrap();

        let root_scarb = dir.join("Scarb.toml");
        fs::write(
            &root_scarb,
            r#"[workspace]
members = [
    "packages/macros",
    "packages/math_lib",
    "packages/vault",
]

[workspace.package]
version = "0.4.2"
edition = "2024_07"

[workspace.dependencies]
starknet = "2.11.4"
snforge_std = "0.39.0"
assert_macros = "2.11.4"
math_lib = { path = "packages/math_lib" }
"#,
        )
        .unwrap();

        fs::write(
            dir.join("packages/macros/Scarb.toml"),
            r#"[package]
name = "ws_macros"
version.workspace = true
edition.workspace = true

[cairo-plugin]
"#,
        )
        .unwrap();

        fs::write(
            dir.join("packages/math_lib/Scarb.toml"),
            r#"[package]
name = "math_lib"
version.workspace = true
edition.workspace = true
experimental-features = ["user_defined_inline_macros"]

[dependencies]
assert_macros.workspace = true
"#,
        )
        .unwrap();
        fs::write(
            dir.join("packages/math_lib/src/lib.cairo"),
            r#"pub macro double_val {
    ($x:expr) => {
        ($x) + ($x)
    };
}

pub fn checked_double(x: u32) -> u32 {
    let res = double_val!(x);
    assert_eq!(res, x + x);
    res
}

#[test]
fn test_checked_double_inline() {
    assert_eq!(checked_double(21), 42);
}
"#,
        )
        .unwrap();

        fs::write(
            dir.join("packages/vault/Scarb.toml"),
            r#"[package]
name = "vault"
version.workspace = true
edition.workspace = true

[dependencies]
starknet.workspace = true
math_lib.workspace = true

[dev_dependencies]
snforge_std.workspace = true

[[target.starknet-contract]]
sierra = true
casm = true
"#,
        )
        .unwrap();
        fs::write(
            dir.join("packages/vault/src/lib.cairo"),
            r#"#[starknet::contract(account)]
pub mod VaultAccount {
    use starknet::account::{AccountContract, Call};

    #[storage]
    struct Storage {
        counter: u32,
    }

    #[abi(embed_v0)]
    impl AccountImpl of AccountContract<ContractState> {
        fn __validate_declare__(self: @ContractState, class_hash: felt252) -> felt252 {
            class_hash
        }
        fn __validate__(ref self: ContractState, calls: Array<Call>) -> felt252 {
            calls.len().into()
        }
        fn __execute__(ref self: ContractState, calls: Array<Call>) -> Array<Span<felt252>> {
            let _ = calls;
            let _ = math_lib::checked_double(5);
            array![]
        }
    }
}
"#,
        )
        .unwrap();

        let report_path = dir.join("migrate-report.json");
        run_migrate(MigrateArgs {
            manifest_path: Some(root_scarb.clone()),
            emit_uc_toml: None,
            dry_run: false,
            remove_legacy: true,
            report_path: Some(report_path.clone()),
            json: false,
        })
        .expect("workspace uc migrate should succeed");

        assert!(!root_scarb.exists(), "root Scarb.toml should be removed");
        assert!(!dir.join("packages/macros/Scarb.toml").exists());
        assert!(!dir.join("packages/math_lib/Scarb.toml").exists());
        assert!(!dir.join("packages/vault/Scarb.toml").exists());
        assert!(dir.join("uc.toml").is_file());
        assert!(dir.join("packages/macros/uc.toml").is_file());
        assert!(dir.join("packages/math_lib/uc.toml").is_file());
        assert!(dir.join("packages/vault/uc.toml").is_file());

        let math_uc: TomlValue = fs::read_to_string(dir.join("packages/math_lib/uc.toml"))
            .unwrap()
            .parse()
            .unwrap();
        assert_eq!(math_uc["project"]["version"].as_str(), Some("0.4.2"));
        assert_eq!(math_uc["project"]["edition"].as_str(), Some("2024_07"));

        let root_uc = dir.join("uc.toml");
        run_check(CheckArgs {
            manifest_path: Some(root_uc.clone()),
            package: None,
            workspace: true,
            offline: true,
            profile: None,
            release: false,
            dev: false,
            json: false,
            report_path: None,
        })
        .expect("uc check --workspace on migrated uc.toml workspace should succeed");

        run_build(BuildArgs {
            common: BuildCommonArgs {
                manifest_path: Some(root_uc.clone()),
                package: None,
                workspace: true,
                features: Vec::new(),
                offline: true,
                release: false,
                profile: None,
            },
            engine: EngineArg::Uc,
            daemon_mode: DaemonModeArg::Off,
            report_path: None,
            record_failure: None,
            json: false,
            plan_only: false,
        })
        .expect("uc build --workspace on migrated uc.toml workspace should succeed");

        run_prove(ProveArgs {
            manifest_path: Some(root_uc.clone()),
            workspace: true,
            ..ProveArgs::default()
        })
        .expect("uc prove --workspace on migrated uc.toml workspace should succeed");

        run_test(TestArgs {
            manifest_path: Some(root_uc.clone()),
            workspace: true,
            ..TestArgs::default()
        })
        .expect("uc test --workspace on migrated uc.toml workspace should succeed");

        run_tree(TreeArgs {
            manifest_path: Some(root_uc.clone()),
            package: None,
            depth: None,
            no_dedupe: false,
            no_dev: false,
            prune: Vec::new(),
            charset: TreeCharsetArg::Utf8,
            json: false,
            report_path: None,
        })
        .expect("uc tree on migrated uc.toml workspace should succeed");

        let _ = fs::remove_dir_all(&dir);
    }
}
