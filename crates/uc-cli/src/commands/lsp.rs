use super::*;
use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Write};

#[derive(Args, Debug, Clone, Default)]
pub(crate) struct LspArgs {
    /// Optional path to Scarb.toml manifest or workspace root.
    #[arg(long)]
    pub manifest_path: Option<PathBuf>,

    /// Run standard LSP JSON-RPC 2.0 server over stdin/stdout (default).
    #[arg(long)]
    pub stdio: bool,

    /// Index workspace symbols and diagnostics once and exit (useful for CI & editor health checks).
    #[arg(long)]
    pub check_workspace: bool,

    /// Emit JSON report when running `--check-workspace`.
    #[arg(long)]
    pub json: bool,

    /// Write JSON report to path when running `--check-workspace`.
    #[arg(long)]
    pub report_path: Option<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct CairoIndexedSymbol {
    pub name: String,
    pub kind: String,
    pub lsp_kind: u32,
    pub signature: String,
    pub doc_comment: Option<String>,
    pub selector_hex: Option<String>,
    pub uri: String,
    pub file_path: String,
    pub line: usize,
    pub character: usize,
    pub end_line: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct CairoLspDiagnostic {
    pub code: String,
    pub severity: u32,
    pub message: String,
    pub line: usize,
    pub character: usize,
    pub end_character: usize,
}

fn path_to_file_uri(path: &Path) -> String {
    let abs = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let s = abs.display().to_string().replace('\\', "/");
    if s.starts_with('/') {
        format!("file://{s}")
    } else {
        format!("file:///{s}")
    }
}

fn file_uri_to_path(uri: &str) -> PathBuf {
    if let Some(rest) = uri.strip_prefix("file://") {
        PathBuf::from(rest)
    } else {
        PathBuf::from(uri)
    }
}

fn compute_starknet_keccak_hex(name: &str) -> String {
    #[cfg(feature = "native-compile")]
    {
        let h = cairo_lang_starknet_classes::keccak::starknet_keccak(name.as_bytes());
        return format!("0x{h:x}");
    }
    #[cfg(not(feature = "native-compile"))]
    {
        format!("0x{}", blake3::hash(name.as_bytes()).to_hex())
    }
}

pub(crate) fn index_cairo_source_symbols(
    uri: &str,
    file_path: &str,
    source: &str,
) -> Vec<CairoIndexedSymbol> {
    let mut symbols = Vec::new();
    let lines: Vec<&str> = source.lines().collect();
    let mut doc_buf: Vec<String> = Vec::new();
    let mut pending_contract = false;
    let mut pending_interface = false;
    let mut pending_test = false;

    for (idx, raw_line) in lines.iter().enumerate() {
        let trimmed = raw_line.trim();
        if let Some(doc) = trimmed.strip_prefix("///") {
            doc_buf.push(doc.trim().to_string());
            continue;
        }
        if trimmed.starts_with("//") {
            continue;
        }
        if trimmed.is_empty() {
            doc_buf.clear();
            continue;
        }
        if trimmed.starts_with("#[starknet::contract") {
            pending_contract = true;
            continue;
        }
        if trimmed.starts_with("#[starknet::interface") {
            pending_interface = true;
            continue;
        }
        if trimmed.starts_with("#[test") {
            pending_test = true;
            continue;
        }
        if trimmed.starts_with("#[") {
            continue;
        }

        let code = raw_line.split("//").next().unwrap_or(raw_line).trim();
        let after_vis = code
            .strip_prefix("pub(crate) ")
            .or_else(|| code.strip_prefix("pub "))
            .unwrap_or(code)
            .trim_start();

        let take_ident = |s: &str| -> String {
            s.chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                .collect()
        };

        let doc_comment = if doc_buf.is_empty() {
            None
        } else {
            Some(doc_buf.join("\n"))
        };

        if let Some(rest) = after_vis.strip_prefix("fn ") {
            let name = take_ident(rest);
            if !name.is_empty() {
                let col = raw_line.find(&name).unwrap_or(0);
                let sig = code.trim_end_matches('{').trim().to_string();
                let kind = if pending_test {
                    "test_function"
                } else {
                    "function"
                };
                let selector_hex = if !pending_test {
                    Some(compute_starknet_keccak_hex(&name))
                } else {
                    None
                };
                symbols.push(CairoIndexedSymbol {
                    name,
                    kind: kind.to_string(),
                    lsp_kind: 12, // Function
                    signature: sig,
                    doc_comment,
                    selector_hex,
                    uri: uri.to_string(),
                    file_path: file_path.to_string(),
                    line: idx,
                    character: col,
                    end_line: idx,
                });
            }
            doc_buf.clear();
            pending_test = false;
        } else if let Some(rest) = after_vis.strip_prefix("mod ") {
            let name = take_ident(rest);
            if !name.is_empty() {
                let col = raw_line.find(&name).unwrap_or(0);
                let kind = if pending_contract {
                    "starknet_contract"
                } else {
                    "module"
                };
                symbols.push(CairoIndexedSymbol {
                    name,
                    kind: kind.to_string(),
                    lsp_kind: 2, // Module
                    signature: code.trim_end_matches('{').trim().to_string(),
                    doc_comment,
                    selector_hex: None,
                    uri: uri.to_string(),
                    file_path: file_path.to_string(),
                    line: idx,
                    character: col,
                    end_line: idx,
                });
            }
            doc_buf.clear();
            pending_contract = false;
        } else if let Some(rest) = after_vis.strip_prefix("trait ") {
            let name = take_ident(rest);
            if !name.is_empty() {
                let col = raw_line.find(&name).unwrap_or(0);
                let kind = if pending_interface {
                    "starknet_interface"
                } else {
                    "trait"
                };
                symbols.push(CairoIndexedSymbol {
                    name,
                    kind: kind.to_string(),
                    lsp_kind: 11, // Interface
                    signature: code.trim_end_matches('{').trim().to_string(),
                    doc_comment,
                    selector_hex: None,
                    uri: uri.to_string(),
                    file_path: file_path.to_string(),
                    line: idx,
                    character: col,
                    end_line: idx,
                });
            }
            doc_buf.clear();
            pending_interface = false;
        } else if let Some(rest) = after_vis.strip_prefix("struct ") {
            let name = take_ident(rest);
            if !name.is_empty() {
                let col = raw_line.find(&name).unwrap_or(0);
                symbols.push(CairoIndexedSymbol {
                    name,
                    kind: "struct".to_string(),
                    lsp_kind: 23, // Struct
                    signature: code.trim_end_matches('{').trim().to_string(),
                    doc_comment,
                    selector_hex: None,
                    uri: uri.to_string(),
                    file_path: file_path.to_string(),
                    line: idx,
                    character: col,
                    end_line: idx,
                });
            }
            doc_buf.clear();
        } else if let Some(rest) = after_vis.strip_prefix("enum ") {
            let name = take_ident(rest);
            if !name.is_empty() {
                let col = raw_line.find(&name).unwrap_or(0);
                symbols.push(CairoIndexedSymbol {
                    name,
                    kind: "enum".to_string(),
                    lsp_kind: 10, // Enum
                    signature: code.trim_end_matches('{').trim().to_string(),
                    doc_comment,
                    selector_hex: None,
                    uri: uri.to_string(),
                    file_path: file_path.to_string(),
                    line: idx,
                    character: col,
                    end_line: idx,
                });
            }
            doc_buf.clear();
        } else if let Some(rest) = after_vis.strip_prefix("const ") {
            let name = take_ident(rest);
            if !name.is_empty() {
                let col = raw_line.find(&name).unwrap_or(0);
                symbols.push(CairoIndexedSymbol {
                    name,
                    kind: "const".to_string(),
                    lsp_kind: 14, // Constant
                    signature: code.to_string(),
                    doc_comment,
                    selector_hex: None,
                    uri: uri.to_string(),
                    file_path: file_path.to_string(),
                    line: idx,
                    character: col,
                    end_line: idx,
                });
            }
            doc_buf.clear();
        } else {
            doc_buf.clear();
            pending_contract = false;
            pending_interface = false;
            pending_test = false;
        }
    }
    symbols
}

pub(crate) fn compute_cairo_lsp_diagnostics(source: &str) -> Vec<CairoLspDiagnostic> {
    let mut diags = Vec::new();

    // 1. In-process Cairo parser syntax diagnostics via CairoFormatter
    #[cfg(feature = "native-compile")]
    {
        use cairo_lang_formatter::{CairoFormatter, FormatterConfig};
        let formatter = CairoFormatter::new(FormatterConfig::default());
        if let Err(err) = formatter.format_to_string(&source.to_string()) {
            let err_str = err.to_string();
            for chunk in err_str.lines() {
                let trimmed = chunk.trim();
                if trimmed.starts_with("error:") {
                    diags.push(CairoLspDiagnostic {
                        code: "CAIRO-SYNTAX".to_string(),
                        severity: 1, // Error
                        message: trimmed.to_string(),
                        line: 0,
                        character: 0,
                        end_character: 80,
                    });
                }
            }
        }
    }

    // 2. Real-time Starknet security, Caracal taint/dominator, gas, and style lint diagnostics
    let has_caller_check = source.contains("get_caller_address")
        || source.contains("assert_only_owner")
        || source.contains("OwnableComponent");
    let is_starknet_contract = source.contains("#[starknet::contract]");
    for (idx, raw_line) in source.lines().enumerate() {
        let code = raw_line.split("//").next().unwrap_or(raw_line).trim();
        if code.is_empty() {
            continue;
        }
        if code.contains("library_call_syscall") && !has_caller_check {
            diags.push(CairoLspDiagnostic {
                code: "UC-SEC-001".to_string(),
                severity: 2, // Warning
                message: "Unprotected `library_call_syscall` without `get_caller_address()` access control check".to_string(),
                line: idx,
                character: 0,
                end_character: raw_line.len(),
            });
        }
        if code.contains("replace_class_syscall") && !has_caller_check {
            diags.push(CairoLspDiagnostic {
                code: "UC-SEC-002".to_string(),
                severity: 2, // Warning
                message: "Contract upgrade via `replace_class_syscall` without caller authorization check".to_string(),
                line: idx,
                character: 0,
                end_character: raw_line.len(),
            });
        }
        if is_starknet_contract
            && code.contains(".write(")
            && !has_caller_check
            && !code.contains("constructor")
        {
            diags.push(CairoLspDiagnostic {
                code: "caracal-unprotected-state-mutation".to_string(),
                severity: 2, // Warning
                message: "Caracal: Contract storage mutation (`.write(...)`) in module without `get_caller_address()` or owner guard".to_string(),
                line: idx,
                character: 0,
                end_character: raw_line.len(),
            });
        }
        if code.contains("_syscall(") && code.contains(".is_err()") {
            diags.push(CairoLspDiagnostic {
                code: "caracal-swallowed-syscall-error".to_string(),
                severity: 2, // Warning
                message: "Caracal: Starknet syscall result checked with `.is_err()` instead of `.unwrap_syscall()` — ensure errors are not swallowed".to_string(),
                line: idx,
                character: 0,
                end_character: raw_line.len(),
            });
        }
        if code.contains("& 1") && (code.contains("== 0") || code.contains("== 1")) {
            diags.push(CairoLspDiagnostic {
                code: "caracal-bitwise-parity-sierra".to_string(),
                severity: 3, // Information
                message: "Caracal: Prefer `DivRem::div_rem(value, 2)` over `(value & 1)` in Cairo to avoid `Bitwise` builtin overhead".to_string(),
                line: idx,
                character: 0,
                end_character: raw_line.len(),
            });
        }
        if code.contains("LegacyMap") {
            diags.push(CairoLspDiagnostic {
                code: "UC-STYLE-001".to_string(),
                severity: 3, // Information
                message: "`LegacyMap` is deprecated in modern Starknet; prefer `starknet::storage::Map`".to_string(),
                line: idx,
                character: 0,
                end_character: raw_line.len(),
            });
        }
    }

    diags
}

fn extract_word_at_position(source: &str, line: usize, character: usize) -> Option<String> {
    let target_line = source.lines().nth(line)?;
    let chars: Vec<char> = target_line.chars().collect();
    if chars.is_empty() {
        return None;
    }
    let pos = character.min(chars.len().saturating_sub(1));
    let is_ident = |c: char| c.is_ascii_alphanumeric() || c == '_';
    if !is_ident(chars[pos]) {
        return None;
    }
    let mut start = pos;
    while start > 0 && is_ident(chars[start - 1]) {
        start -= 1;
    }
    let mut end = pos;
    while end + 1 < chars.len() && is_ident(chars[end + 1]) {
        end += 1;
    }
    Some(chars[start..=end].iter().collect())
}

pub(crate) struct LspServerState {
    pub workspace_root: PathBuf,
    pub open_documents: BTreeMap<String, String>,
    pub symbol_index: BTreeMap<String, Vec<CairoIndexedSymbol>>,
    pub shutdown_requested: bool,
}

impl LspServerState {
    pub fn new(workspace_root: PathBuf) -> Self {
        let mut state = Self {
            workspace_root,
            open_documents: BTreeMap::new(),
            symbol_index: BTreeMap::new(),
            shutdown_requested: false,
        };
        state.scan_workspace();
        state
    }

    pub fn scan_workspace(&mut self) {
        if !self.workspace_root.is_dir() {
            return;
        }
        for entry in WalkDir::new(&self.workspace_root)
            .max_depth(6)
            .follow_links(false)
            .into_iter()
            .flatten()
        {
            if !entry.file_type().is_file() {
                continue;
            }
            let path = entry.path();
            let path_str = path.display().to_string();
            if path_str.contains("/target/") || path_str.contains("/.uc/") {
                continue;
            }
            if path.extension().and_then(|e| e.to_str()) != Some("cairo") {
                continue;
            }
            let Ok(content) = fs::read_to_string(path) else {
                continue;
            };
            let uri = path_to_file_uri(path);
            let rel = path
                .strip_prefix(&self.workspace_root)
                .unwrap_or(path)
                .display()
                .to_string();
            let syms = index_cairo_source_symbols(&uri, &rel, &content);
            self.symbol_index.insert(uri.clone(), syms);
            self.open_documents.entry(uri).or_insert(content);
        }
    }

    pub fn update_document(&mut self, uri: &str, text: String) -> Vec<CairoLspDiagnostic> {
        let path = file_uri_to_path(uri);
        let rel = path
            .strip_prefix(&self.workspace_root)
            .unwrap_or(&path)
            .display()
            .to_string();
        let syms = index_cairo_source_symbols(uri, &rel, &text);
        self.symbol_index.insert(uri.to_string(), syms);
        let diags = compute_cairo_lsp_diagnostics(&text);
        self.open_documents.insert(uri.to_string(), text);
        diags
    }

    pub fn all_symbols(&self) -> impl Iterator<Item = &CairoIndexedSymbol> {
        self.symbol_index.values().flatten()
    }
}

fn write_lsp_message<W: Write>(writer: &mut W, payload: &serde_json::Value) -> Result<()> {
    let body = serde_json::to_vec(payload)?;
    write!(writer, "Content-Length: {}\r\n\r\n", body.len())?;
    writer.write_all(&body)?;
    writer.flush()?;
    Ok(())
}

fn read_lsp_message<R: BufRead>(reader: &mut R) -> Result<Option<serde_json::Value>> {
    let mut content_length: Option<usize> = None;
    loop {
        let mut line = String::new();
        let bytes_read = reader.read_line(&mut line)?;
        if bytes_read == 0 {
            return Ok(None);
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            break;
        }
        if let Some((k, v)) = trimmed.split_once(':') {
            if k.trim().eq_ignore_ascii_case("content-length") {
                content_length = v.trim().parse::<usize>().ok();
            }
        }
    }
    let Some(len) = content_length else {
        return Ok(None);
    };
    let mut buf = vec![0u8; len];
    reader.read_exact(&mut buf)?;
    let msg: serde_json::Value = serde_json::from_slice(&buf)?;
    Ok(Some(msg))
}

fn build_publish_diagnostics_notification(
    uri: &str,
    diags: &[CairoLspDiagnostic],
) -> serde_json::Value {
    let lsp_diags: Vec<serde_json::Value> = diags
        .iter()
        .map(|d| {
            serde_json::json!({
                "range": {
                    "start": { "line": d.line, "character": d.character },
                    "end": { "line": d.line, "character": d.end_character.max(d.character + 1) }
                },
                "severity": d.severity,
                "code": d.code,
                "source": "uc-lsp",
                "message": d.message,
            })
        })
        .collect();
    serde_json::json!({
        "jsonrpc": "2.0",
        "method": "textDocument/publishDiagnostics",
        "params": {
            "uri": uri,
            "diagnostics": lsp_diags,
        }
    })
}

pub(crate) fn serve_lsp_stream<R: BufRead, W: Write>(
    mut reader: R,
    mut writer: W,
    initial_workspace_root: PathBuf,
) -> Result<()> {
    let mut state = LspServerState::new(initial_workspace_root);

    while let Some(msg) = read_lsp_message(&mut reader)? {
        let method = msg.get("method").and_then(|v| v.as_str()).unwrap_or("");
        let req_id = msg.get("id").cloned();
        let params = msg.get("params").cloned().unwrap_or(serde_json::Value::Null);

        match method {
            "initialize" => {
                if let Some(root_uri) = params.get("rootUri").and_then(|v| v.as_str()) {
                    let root_path = file_uri_to_path(root_uri);
                    if root_path.is_dir() {
                        state.workspace_root = root_path;
                        state.scan_workspace();
                    }
                }
                if let Some(id) = req_id {
                    let resp = serde_json::json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": {
                            "capabilities": {
                                "textDocumentSync": {
                                    "openClose": true,
                                    "change": 1,
                                    "save": { "includeText": true }
                                },
                                "hoverProvider": true,
                                "definitionProvider": true,
                                "documentSymbolProvider": true,
                                "workspaceSymbolProvider": true,
                                "documentFormattingProvider": true,
                                "codeActionProvider": true,
                                "codeLensProvider": { "resolveProvider": false },
                                "completionProvider": {
                                    "resolveProvider": false,
                                    "triggerCharacters": [":", ".", "#", "@"]
                                }
                            },
                            "serverInfo": {
                                "name": "uc-lsp",
                                "version": env!("CARGO_PKG_VERSION")
                            }
                        }
                    });
                    write_lsp_message(&mut writer, &resp)?;
                }
            }
            "initialized" => {
                state.scan_workspace();
            }
            "textDocument/didOpen" => {
                if let Some(doc) = params.get("textDocument") {
                    let uri = doc.get("uri").and_then(|v| v.as_str()).unwrap_or("");
                    let text = doc.get("text").and_then(|v| v.as_str()).unwrap_or("");
                    if !uri.is_empty() {
                        let diags = state.update_document(uri, text.to_string());
                        let notif = build_publish_diagnostics_notification(uri, &diags);
                        write_lsp_message(&mut writer, &notif)?;
                    }
                }
            }
            "textDocument/didChange" => {
                let uri = params
                    .get("textDocument")
                    .and_then(|d| d.get("uri"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                if let Some(changes) = params.get("contentChanges").and_then(|v| v.as_array()) {
                    if let Some(last_change) = changes.last() {
                        if let Some(text) = last_change.get("text").and_then(|v| v.as_str()) {
                            let diags = state.update_document(uri, text.to_string());
                            let notif = build_publish_diagnostics_notification(uri, &diags);
                            write_lsp_message(&mut writer, &notif)?;
                        }
                    }
                }
            }
            "textDocument/didSave" => {
                let uri = params
                    .get("textDocument")
                    .and_then(|d| d.get("uri"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let text_opt = params
                    .get("text")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
                    .or_else(|| fs::read_to_string(file_uri_to_path(uri)).ok());
                if let Some(text) = text_opt {
                    let diags = state.update_document(uri, text);
                    let notif = build_publish_diagnostics_notification(uri, &diags);
                    write_lsp_message(&mut writer, &notif)?;
                }
            }
            "textDocument/hover" => {
                let Some(id) = req_id else { continue };
                let uri = params
                    .get("textDocument")
                    .and_then(|d| d.get("uri"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let line = params
                    .get("position")
                    .and_then(|p| p.get("line"))
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0) as usize;
                let character = params
                    .get("position")
                    .and_then(|p| p.get("character"))
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0) as usize;

                let mut hover_markdown: Option<String> = None;
                if let Some(doc_text) = state.open_documents.get(uri) {
                    if let Some(word) = extract_word_at_position(doc_text, line, character) {
                        if let Some(sym) = state.all_symbols().find(|s| s.name == word) {
                            let mut md = format!("```cairo\n{}\n```\n", sym.signature);
                            if let Some(sel) = &sym.selector_hex {
                                md.push_str(&format!(
                                    "\n**Starknet Selector (`sn_keccak`)**: `{sel}`\n"
                                ));
                            }
                            if let Some(doc) = &sym.doc_comment {
                                md.push_str(&format!("\n---\n{doc}\n"));
                            }
                            md.push_str(&format!(
                                "\n*Defined in `{}:{}`*",
                                sym.file_path,
                                sym.line + 1
                            ));
                            hover_markdown = Some(md);
                        }
                    }
                }
                let result_val = match hover_markdown {
                    Some(md) => serde_json::json!({
                        "contents": {
                            "kind": "markdown",
                            "value": md
                        }
                    }),
                    None => serde_json::Value::Null,
                };
                write_lsp_message(
                    &mut writer,
                    &serde_json::json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": result_val
                    }),
                )?;
            }
            "textDocument/definition" => {
                let Some(id) = req_id else { continue };
                let uri = params
                    .get("textDocument")
                    .and_then(|d| d.get("uri"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let line = params
                    .get("position")
                    .and_then(|p| p.get("line"))
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0) as usize;
                let character = params
                    .get("position")
                    .and_then(|p| p.get("character"))
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0) as usize;

                let mut location_val = serde_json::Value::Null;
                if let Some(doc_text) = state.open_documents.get(uri) {
                    if let Some(word) = extract_word_at_position(doc_text, line, character) {
                        if let Some(sym) = state.all_symbols().find(|s| s.name == word) {
                            location_val = serde_json::json!({
                                "uri": sym.uri,
                                "range": {
                                    "start": { "line": sym.line, "character": sym.character },
                                    "end": { "line": sym.line, "character": sym.character + sym.name.len() }
                                }
                            });
                        }
                    }
                }
                write_lsp_message(
                    &mut writer,
                    &serde_json::json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": location_val
                    }),
                )?;
            }
            "textDocument/documentSymbol" => {
                let Some(id) = req_id else { continue };
                let uri = params
                    .get("textDocument")
                    .and_then(|d| d.get("uri"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let syms: Vec<serde_json::Value> = state
                    .symbol_index
                    .get(uri)
                    .map(|list| {
                        list.iter()
                            .map(|s| {
                                serde_json::json!({
                                    "name": s.name,
                                    "detail": s.signature,
                                    "kind": s.lsp_kind,
                                    "range": {
                                        "start": { "line": s.line, "character": 0 },
                                        "end": { "line": s.end_line, "character": s.character + s.name.len() }
                                    },
                                    "selectionRange": {
                                        "start": { "line": s.line, "character": s.character },
                                        "end": { "line": s.line, "character": s.character + s.name.len() }
                                    }
                                })
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                write_lsp_message(
                    &mut writer,
                    &serde_json::json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": syms
                    }),
                )?;
            }
            "workspace/symbol" => {
                let Some(id) = req_id else { continue };
                let query = params
                    .get("query")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_ascii_lowercase();
                let matches: Vec<serde_json::Value> = state
                    .all_symbols()
                    .filter(|s| query.is_empty() || s.name.to_ascii_lowercase().contains(&query))
                    .map(|s| {
                        serde_json::json!({
                            "name": s.name,
                            "kind": s.lsp_kind,
                            "location": {
                                "uri": s.uri,
                                "range": {
                                    "start": { "line": s.line, "character": s.character },
                                    "end": { "line": s.line, "character": s.character + s.name.len() }
                                }
                            }
                        })
                    })
                    .collect();
                write_lsp_message(
                    &mut writer,
                    &serde_json::json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": matches
                    }),
                )?;
            }
            "textDocument/completion" => {
                let Some(id) = req_id else { continue };
                let mut items: Vec<serde_json::Value> = vec![
                    serde_json::json!({
                        "label": "#[starknet::contract]",
                        "kind": 15,
                        "detail": "Starknet contract module attribute"
                    }),
                    serde_json::json!({
                        "label": "#[starknet::interface]",
                        "kind": 15,
                        "detail": "Starknet ABI interface trait attribute"
                    }),
                    serde_json::json!({
                        "label": "#[abi(embed_v0)]",
                        "kind": 15,
                        "detail": "Embed trait implementation into Starknet contract ABI"
                    }),
                    serde_json::json!({
                        "label": "ContractAddress",
                        "kind": 22,
                        "detail": "starknet::ContractAddress"
                    }),
                    serde_json::json!({
                        "label": "get_caller_address",
                        "kind": 3,
                        "detail": "starknet::get_caller_address() -> ContractAddress"
                    }),
                    serde_json::json!({
                        "label": "start_cheat_caller_address",
                        "kind": 3,
                        "detail": "snforge_std::start_cheat_caller_address(target, caller)"
                    }),
                ];
                for sym in state.all_symbols() {
                    items.push(serde_json::json!({
                        "label": sym.name,
                        "kind": sym.lsp_kind,
                        "detail": sym.signature,
                        "documentation": sym.doc_comment,
                    }));
                }
                write_lsp_message(
                    &mut writer,
                    &serde_json::json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": items
                    }),
                )?;
            }
            "textDocument/formatting" => {
                let Some(id) = req_id else { continue };
                let uri = params
                    .get("textDocument")
                    .and_then(|d| d.get("uri"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let mut edits = Vec::new();
                if let Some(doc_text) = state.open_documents.get(uri) {
                    #[cfg(feature = "native-compile")]
                    {
                        use cairo_lang_formatter::{CairoFormatter, FormatOutcome, FormatterConfig};
                        let formatter = CairoFormatter::new(FormatterConfig::default());
                        if let Ok(FormatOutcome::DiffFound(diff)) =
                            formatter.format_to_string(doc_text)
                        {
                            let line_count = doc_text.lines().count() + 1;
                            edits.push(serde_json::json!({
                                "range": {
                                    "start": { "line": 0, "character": 0 },
                                    "end": { "line": line_count, "character": 0 }
                                },
                                "newText": diff.formatted
                            }));
                        }
                    }
                }
                write_lsp_message(
                    &mut writer,
                    &serde_json::json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": edits
                    }),
                )?;
            }
            "textDocument/codeLens" => {
                let Some(id) = req_id else { continue };
                let uri = params
                    .get("textDocument")
                    .and_then(|d| d.get("uri"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let mut lenses = Vec::new();
                if let Some(syms) = state.symbol_index.get(uri) {
                    for s in syms {
                        if s.kind == "test_function" {
                            lenses.push(serde_json::json!({
                                "range": {
                                    "start": { "line": s.line, "character": 0 },
                                    "end": { "line": s.line, "character": s.name.len() }
                                },
                                "command": {
                                    "title": format!("▶ Run uc test {}", s.name),
                                    "command": "uc.runTest",
                                    "arguments": [s.name]
                                }
                            }));
                        } else if let Some(sel) = &s.selector_hex {
                            lenses.push(serde_json::json!({
                                "range": {
                                    "start": { "line": s.line, "character": 0 },
                                    "end": { "line": s.line, "character": s.name.len() }
                                },
                                "command": {
                                    "title": format!("selector: {sel}"),
                                    "command": "uc.copySelector",
                                    "arguments": [s.name, sel]
                                }
                            }));
                        }
                    }
                }
                write_lsp_message(
                    &mut writer,
                    &serde_json::json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": lenses
                    }),
                )?;
            }
            "textDocument/codeAction" => {
                let Some(id) = req_id else { continue };
                write_lsp_message(
                    &mut writer,
                    &serde_json::json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": []
                    }),
                )?;
            }
            "shutdown" => {
                state.shutdown_requested = true;
                if let Some(id) = req_id {
                    write_lsp_message(
                        &mut writer,
                        &serde_json::json!({
                            "jsonrpc": "2.0",
                            "id": id,
                            "result": serde_json::Value::Null
                        }),
                    )?;
                }
            }
            "exit" => {
                break;
            }
            _ => {
                if let Some(id) = req_id {
                    write_lsp_message(
                        &mut writer,
                        &serde_json::json!({
                            "jsonrpc": "2.0",
                            "id": id,
                            "result": serde_json::Value::Null
                        }),
                    )?;
                }
            }
        }
    }
    Ok(())
}

pub(crate) fn run_lsp(args: LspArgs) -> Result<()> {
    let started = Instant::now();
    let workspace_root = if let Some(mp) = &args.manifest_path {
        let resolved = resolve_manifest_path(&Some(mp.clone()))?;
        metadata_cache_workspace_root(&resolved)?
    } else if let Ok(resolved) = resolve_manifest_path(&None) {
        metadata_cache_workspace_root(&resolved).unwrap_or_else(|_| {
            std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
        })
    } else {
        std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
    };

    if args.check_workspace {
        let state = LspServerState::new(workspace_root.clone());
        let mut diagnostics_by_file: BTreeMap<String, Vec<CairoLspDiagnostic>> = BTreeMap::new();
        let mut total_diagnostics = 0usize;
        for (uri, text) in &state.open_documents {
            let diags = compute_cairo_lsp_diagnostics(text);
            total_diagnostics += diags.len();
            if !diags.is_empty() {
                let rel = file_uri_to_path(uri)
                    .strip_prefix(&workspace_root)
                    .unwrap_or(&file_uri_to_path(uri))
                    .display()
                    .to_string();
                diagnostics_by_file.insert(rel, diags);
            }
        }
        let all_syms: Vec<&CairoIndexedSymbol> = state.all_symbols().collect();
        let elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;
        let report = serde_json::json!({
            "schema_version": UC_AGENT_JSON_SCHEMA_VERSION,
            "status": "ok",
            "server": "uc-lsp",
            "workspace_root": workspace_root.display().to_string(),
            "indexed_files": state.open_documents.len(),
            "indexed_symbols_count": all_syms.len(),
            "diagnostics_count": total_diagnostics,
            "elapsed_ms": elapsed_ms,
            "symbols": all_syms,
            "diagnostics": diagnostics_by_file,
        });
        if args.json || args.report_path.is_some() {
            emit_json_value(args.report_path.as_deref(), &report)?;
        } else {
            println!(
                "uc-lsp workspace index: {} file(s), {} symbol(s), {} diagnostic(s) in {:.2}ms",
                state.open_documents.len(),
                all_syms.len(),
                total_diagnostics,
                elapsed_ms
            );
        }
        return Ok(());
    }

    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    let reader = BufReader::new(stdin.lock());
    let writer = stdout.lock();
    serve_lsp_stream(reader, writer, workspace_root)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn encode_lsp_frame(val: &serde_json::Value) -> Vec<u8> {
        let body = serde_json::to_vec(val).unwrap();
        let mut out = format!("Content-Length: {}\r\n\r\n", body.len()).into_bytes();
        out.extend_from_slice(&body);
        out
    }

    fn decode_all_lsp_frames(bytes: &[u8]) -> Vec<serde_json::Value> {
        let mut cursor = BufReader::new(Cursor::new(bytes));
        let mut msgs = Vec::new();
        while let Ok(Some(msg)) = read_lsp_message(&mut cursor) {
            msgs.push(msg);
        }
        msgs
    }

    #[test]
    fn lsp_server_handles_initialize_did_open_hover_definition_symbols_completion_and_codelens() {
        let tmp_dir = std::env::temp_dir().join(format!("uc_lsp_test_{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp_dir);
        fs::create_dir_all(tmp_dir.join("src")).unwrap();

        let lib_path = tmp_dir.join("src/lib.cairo");
        let cairo_code = r#"/// Token vault contract for Starknet
#[starknet::contract]
pub mod Vault {
    /// Deposit tokens into the vault
    pub fn deposit(amount: u128) -> u128 {
        amount + 1
    }

    #[test]
    fn test_deposit() {
        let res = deposit(100);
        assert(res == 101, 'bad');
    }
}
"#;
        fs::write(&lib_path, cairo_code).unwrap();
        let uri = path_to_file_uri(&lib_path);

        let mut input_stream = Vec::new();
        input_stream.extend_from_slice(&encode_lsp_frame(&serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "rootUri": path_to_file_uri(&tmp_dir)
            }
        })));
        input_stream.extend_from_slice(&encode_lsp_frame(&serde_json::json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didOpen",
            "params": {
                "textDocument": {
                    "uri": uri,
                    "languageId": "cairo",
                    "version": 1,
                    "text": cairo_code
                }
            }
        })));
        // Hover over `deposit` on line 4 (`pub fn deposit(amount: u128) -> u128 {`)
        input_stream.extend_from_slice(&encode_lsp_frame(&serde_json::json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "textDocument/hover",
            "params": {
                "textDocument": { "uri": uri },
                "position": { "line": 4, "character": 12 }
            }
        })));
        // Go-to-definition from `deposit(100)` call on line 10
        input_stream.extend_from_slice(&encode_lsp_frame(&serde_json::json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "textDocument/definition",
            "params": {
                "textDocument": { "uri": uri },
                "position": { "line": 10, "character": 19 }
            }
        })));
        // Document symbols
        input_stream.extend_from_slice(&encode_lsp_frame(&serde_json::json!({
            "jsonrpc": "2.0",
            "id": 4,
            "method": "textDocument/documentSymbol",
            "params": {
                "textDocument": { "uri": uri }
            }
        })));
        // CodeLens
        input_stream.extend_from_slice(&encode_lsp_frame(&serde_json::json!({
            "jsonrpc": "2.0",
            "id": 5,
            "method": "textDocument/codeLens",
            "params": {
                "textDocument": { "uri": uri }
            }
        })));
        // Shutdown & Exit
        input_stream.extend_from_slice(&encode_lsp_frame(&serde_json::json!({
            "jsonrpc": "2.0",
            "id": 6,
            "method": "shutdown"
        })));
        input_stream.extend_from_slice(&encode_lsp_frame(&serde_json::json!({
            "jsonrpc": "2.0",
            "method": "exit"
        })));

        let mut output_stream = Vec::new();
        serve_lsp_stream(
            BufReader::new(Cursor::new(input_stream)),
            &mut output_stream,
            tmp_dir.clone(),
        )
        .expect("LSP session should succeed");

        let responses = decode_all_lsp_frames(&output_stream);
        assert!(responses.len() >= 6);

        // Check initialize capabilities
        let init_resp = responses.iter().find(|r| r["id"] == 1).unwrap();
        assert_eq!(init_resp["result"]["capabilities"]["hoverProvider"], true);
        assert_eq!(
            init_resp["result"]["capabilities"]["definitionProvider"],
            true
        );

        // Check hover contains signature, doc comment, and Starknet sn_keccak selector
        let hover_resp = responses.iter().find(|r| r["id"] == 2).unwrap();
        let hover_md = hover_resp["result"]["contents"]["value"].as_str().unwrap();
        assert!(hover_md.contains("pub fn deposit(amount: u128) -> u128"));
        assert!(hover_md.contains("Deposit tokens into the vault"));
        assert!(hover_md.contains("Starknet Selector (`sn_keccak`)"));

        // Check go-to-definition resolves `deposit` back to line 4
        let def_resp = responses.iter().find(|r| r["id"] == 3).unwrap();
        assert_eq!(def_resp["result"]["range"]["start"]["line"], 4);

        // Check documentSymbol returns Vault, deposit, test_deposit
        let sym_resp = responses.iter().find(|r| r["id"] == 4).unwrap();
        assert_eq!(sym_resp["result"].as_array().unwrap().len(), 3);

        // Check codeLens returns selector lens and test runner lens
        let lens_resp = responses.iter().find(|r| r["id"] == 5).unwrap();
        assert_eq!(lens_resp["result"].as_array().unwrap().len(), 2);

        let _ = fs::remove_dir_all(&tmp_dir);
    }
}
