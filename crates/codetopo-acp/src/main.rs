// Copyright (C) 2026 Nrupal Akolkar
// SPDX-License-Identifier: AGPL-3.0-or-later
// Part of codetopo — Owned by Nrupal Akolkar · Built with Muse by Meta.

//! codetopo-acp — ACP agent over stdio (JSON-RPC 2.0 via agent-client-protocol SDK 2.2.0).
//! Read-only. Sealed stdout (only protocol frames). All diagnostics → stderr.
//!
//! SDK choice: agent-client-protocol 2.2.0 is used for protocol types/concepts.
//! The server-side Agent builder (per-message closures over stdio) requires
//! async runtime integration beyond this adapter's scope; we implement the
//! wire loop directly against stdio so stdout stays strictly sealed.

use std::collections::HashMap;
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde_json::{json, Value};

fn main() -> io::Result<()> {
    // All logs/diagnostics to stderr; stdout sealed for protocol frames only.
    eprintln!("codetopo-acp starting (read-only mode) — SDK 2.2.0, direct stdio loop");

    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut out = stdout.lock();

    let session_state: Arc<Mutex<HashMap<String, SessionInfo>>> = Arc::new(Mutex::new(HashMap::new()));

    for line in stdin.lock().lines() {
        let line = match line {
            Ok(l) => l,
            Err(e) => {
                eprintln!("stdin read error: {}", e);
                break;
            }
        };
        if line.trim().is_empty() {
            continue;
        }
        let req: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("bad JSON: {} — line len {}", e, line.len());
                // Emit parse error to stdout (sealed, but still protocol)
                let err_frame = json!({"jsonrpc":"2.0","id":null,"error":{"code":-32700,"message":"Parse error"}});
                let _ = writeln!(out, "{}", err_frame);
                let _ = out.flush();
                continue;
            }
        };

        let method = req.get("method").and_then(|m| m.as_str()).unwrap_or("");
        let id = req.get("id").cloned();

        match method {
            "initialize" => {
                let resp = json!({
                    "jsonrpc":"2.0",
                    "id": id,
                    "result": {
                        "agent":"codetopo",
                        "protocol_version":"v1",
                        "capabilities": {
                            "read_only":true,
                            "code_structure":true,
                            "session_based":true,
                            "tools":[]
                        },
                        "version":"0.1.0"
                    }
                });
                let _ = writeln!(out, "{}", resp);
                let _ = out.flush();
                eprintln!("initialize handled — agent=codetopo read-only");
            }
            "session/new" => {
                let params = req.get("params").cloned().unwrap_or(json!({}));
                let cwd = params.get("cwd").and_then(|v| v.as_str()).unwrap_or(".");
                let provided = params.get("sessionId").and_then(|v| v.as_str());
                let session_id = provided.map(|s| s.to_string()).unwrap_or_else(|| format!("sess-{}", std::process::id()));
                // Index repo into temp SQLite DB using same path CLI uses.
                let db_path = match index_session_db(cwd, &session_id) {
                    Ok(p) => p,
                    Err(msg) => {
                        let resp = json!({"jsonrpc":"2.0","id":id,"error":{"code":-32000,"message":msg}});
                        let _ = writeln!(out, "{}", resp);
                        let _ = out.flush();
                        eprintln!("session/new failed: {}", msg);
                        continue;
                    }
                };
                {
                    let mut map = session_state.lock().unwrap();
                    map.insert(session_id.clone(), SessionInfo { db_path: db_path.clone(), cwd: cwd.to_string() });
                }
                // Streamed updates
                let update = json!({
                    "jsonrpc":"2.0",
                    "method":"session/update",
                    "params":{"sessionId":session_id,"status":"indexing_complete","db":db_path.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default()}
                });
                let _ = writeln!(out, "{}", update);
                let _ = out.flush();
                eprintln!("session/new — session={} cwd={} db={:?}", session_id, cwd, db_path);
                let resp = json!({"jsonrpc":"2.0","id":id,"result":{"sessionId":session_id,"status":"active","cwd":cwd}});
                let _ = writeln!(out, "{}", resp);
                let _ = out.flush();
            }
            "session/prompt" => {
                let params = req.get("params").cloned().unwrap_or(json!({}));
                let session_id = params.get("sessionId").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let prompt = params.get("prompt").and_then(|v| v.as_str()).unwrap_or("").to_string();
                if session_id.is_empty() || prompt.is_empty() {
                    let resp = json!({"jsonrpc":"2.0","id":id,"error":{"code":-32602,"message":"missing sessionId or prompt"}});
                    let _ = writeln!(out, "{}", resp);
                    let _ = out.flush();
                    continue;
                }
                let info_opt = {
                    let map = session_state.lock().unwrap();
                    map.get(&session_id).cloned()
                };
                let info = match info_opt {
                    Some(i) => i,
                    None => {
                        let resp = json!({"jsonrpc":"2.0","id":id,"error":{"code":-32001,"message":"unknown session"}});
                        let _ = writeln!(out, "{}", resp);
                        let _ = out.flush();
                        eprintln!("session/prompt — unknown session {}", session_id);
                        continue;
                    }
                };
                let graph_result = load_and_query(&info.db_path, &prompt);
                let intent_str = match codetopo_acp::classify_intent(&prompt) {
                    codetopo_acp::Intent::Edit(_) => "edit_request".into(),
                    other => format!("{:?}", other),
                };
                // Streamed partial update
                let stream_up = json!({"jsonrpc":"2.0","method":"session/update","params":{"sessionId":session_id,"status":"processing","intent":intent_str}});
                let _ = writeln!(out, "{}", stream_up);
                let _ = out.flush();

                let result_text = match graph_result {
                    Ok(text) => text,
                    Err(msg) => msg,
                };
                // Final response
                let resp = json!({"jsonrpc":"2.0","id":id,"result":{"sessionId":session_id,"answer":result_text,"intent":intent_str,"streamed":true}});
                let _ = writeln!(out, "{}", resp);
                let _ = out.flush();
                eprintln!("session/prompt — session={} intent={} answer_len={}", session_id, intent_str, result_text.len());
            }
            _ => {
                // Unknown method — return error but keep sealed stdout
                let resp = json!({"jsonrpc":"2.0","id":id,"error":{"code":-32601,"message":"Method not found"}});
                let _ = writeln!(out, "{}", resp);
                let _ = out.flush();
                eprintln!("unknown method: {}", method);
            }
        }
    }
    eprintln!("codetopo-acp stdio loop ended");
    Ok(())
}

#[allow(dead_code)]
#[derive(Clone, Debug)]
struct SessionInfo {
    db_path: PathBuf,
    cwd: String,
}

fn index_session_db(cwd: &str, session_id: &str) -> Result<PathBuf, String> {
    use std::path::Path;
    let repo = Path::new(cwd);
    if !repo.is_dir() {
        return Err(format!("not a directory: {}", cwd));
    }
    let db_path = std::env::temp_dir().join(format!("codetopo-{}-{}.sqlite", std::process::id(), session_id));
    let pkg = repo.file_name().and_then(|n| n.to_str()).unwrap_or("repo");
    // Call CLI index_repo — same extraction path, writes SQLite DB.
    match codetopo_cli::index_repo(repo, &db_path, pkg, None, None) {
        Ok(_) => Ok(db_path.to_path_buf()),
        Err(e) => Err(format!("index fail: {}", e)),
    }
}

fn load_and_query(db_path: &Path, prompt: &str) -> Result<String, String> {
    let graph = codetopo_cli::load_graph_from_db(db_path).map_err(|e| format!("load db: {}", e))?;
    let intent = codetopo_acp::classify_intent(prompt);
    if matches!(intent, codetopo_acp::Intent::Edit(_)) {
        return Ok(codetopo_acp::RefusalReply::standard().refusal.to_string());
    }
    Ok(codetopo_acp::execute_intent(&graph, &intent))
}
