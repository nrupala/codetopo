// Copyright (C) 2026 Nrupal Akolkar
// SPDX-License-Identifier: AGPL-3.0-or-later
// Part of codetopo — Owned by Nrupal Akolkar (https://nrupalakolkar.com) · Built with Muse by Meta (https://muse.ai/join) — join Muse, then redeem code 101EEK in Settings within 48 hours of joining and we both earn tokens.

//! codetopo-acp — ACP agent over stdio (newline-delimited JSON-RPC 2.0).
//! Read-only. Sealed stdout (only protocol frames). All diagnostics → stderr.
//!
//! SDK choice: none. Direct wire-protocol implementation against the public
//! ACP spec (newline-delimited JSON-RPC 2.0 over stdio; no Content-Length
//! headers, stderr free-form logging, sealed stdout). Nothing vendored, nothing
//! copied.

use std::collections::HashMap;
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde_json::{json, Value};

fn main() -> io::Result<()> {
    // Terminal Auth entry point: ACP clients re-run this binary with
    // `login` in an interactive terminal to run the proof-verification
    // setup (see codetopo_acp::trust). A zero exit status is success.
    // Any other argument is a usage error — the protocol loop itself
    // takes no arguments.
    match std::env::args().nth(1).as_deref() {
        Some("login") | Some("--setup") => {
            std::process::exit(codetopo_acp::trust::run_trust_setup());
        }
        Some(other) => {
            eprintln!("codetopo-acp: unknown argument '{other}' — usage: codetopo-acp [login]");
            std::process::exit(2);
        }
        None => {}
    }

    // All logs/diagnostics to stderr; stdout sealed for protocol frames only.
    eprintln!("codetopo-acp starting (read-only mode) — direct ACP wire loop, no SDK");

    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut out = stdout.lock();

    let session_state: Arc<Mutex<HashMap<String, SessionInfo>>> = Arc::new(Mutex::new(HashMap::new()));
    let mut session_counter: u64 = 0;
    // Sessions whose in-flight turn was cancelled. Turns here are
    // synchronous (one line at a time), so a cancel can only land
    // *between* turns; it is recorded so the log tells the truth,
    // and it never destroys the session — cancel ends a turn in
    // ACP, not the session.
    let mut cancelled: std::collections::HashSet<String> = std::collections::HashSet::new();

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
        // A frame with no `id` is a notification: it is never
        // answered, whatever it carries.
        let is_notification = req.get("id").is_none();

        match method {
            "initialize" => {
                let params = req.get("params").cloned().unwrap_or(json!({}));
                let resp = json!({
                    "jsonrpc":"2.0",
                    "id": id,
                    "result": codetopo_acp::initialize_result(&params)
                });
                let _ = writeln!(out, "{}", resp);
                let _ = out.flush();
                eprintln!("initialize handled — agent=codetopo read-only");
            }
            "session/new" => {
                let params = req.get("params").cloned().unwrap_or(json!({}));
                let cwd = params.get("cwd").and_then(|v| v.as_str()).unwrap_or(".");
                // The agent mints the session id (the spec's
                // session/new takes cwd + mcpServers, no id).
                session_counter += 1;
                let session_id = format!("sess-{}-{}", std::process::id(), session_counter);
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
                    map.insert(session_id.clone(), SessionInfo { db_path: db_path.clone() });
                }
                eprintln!("session/new — session={} cwd={} db={:?}", session_id, cwd, db_path);
                // The spec result is exactly { sessionId }.
                let resp = json!({"jsonrpc":"2.0","id":id,"result":{"sessionId":session_id}});
                let _ = writeln!(out, "{}", resp);
                let _ = out.flush();
            }
            "session/prompt" => {
                let params = req.get("params").cloned().unwrap_or(json!({}));
                let session_id = params.get("sessionId").and_then(|v| v.as_str()).unwrap_or("").to_string();
                // ACP prompts are content-block arrays, not strings.
                let prompt = codetopo_acp::prompt_text(params.get("prompt").unwrap_or(&Value::Null));
                if session_id.is_empty() || prompt.is_empty() {
                    let resp = json!({"jsonrpc":"2.0","id":id,"error":{"code":-32602,"message":"missing sessionId or prompt text"}});
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
                if cancelled.remove(&session_id) {
                    eprintln!("session/prompt — session={} had a cancelled turn; serving the new prompt", session_id);
                }
                let is_edit = matches!(codetopo_acp::classify_intent(&prompt), codetopo_acp::Intent::Edit(_));
                let result_text = match load_and_query(&info.db_path, &prompt) {
                    Ok(text) => text,
                    Err(msg) => msg,
                };
                // Stream the answer as one agent_message_chunk
                // (session/update), then answer with the spec's
                // stopReason — "refusal" for edit requests this
                // read-only agent will not perform.
                let update = json!({
                    "jsonrpc":"2.0",
                    "method":"session/update",
                    "params": codetopo_acp::message_chunk(&session_id, &result_text),
                });
                let _ = writeln!(out, "{}", update);
                let _ = out.flush();
                let stop_reason = if is_edit { "refusal" } else { "end_turn" };
                let resp = json!({"jsonrpc":"2.0","id":id,"result":{"stopReason":stop_reason}});
                let _ = writeln!(out, "{}", resp);
                let _ = out.flush();
                eprintln!("session/prompt — session={} edit={} answer_len={}", session_id, is_edit, result_text.len());
            }
            "session/cancel" => {
                let params = req.get("params").cloned().unwrap_or(json!({}));
                if let Some(sid) = params.get("sessionId").and_then(|v| v.as_str()) {
                    cancelled.insert(sid.to_string());
                    eprintln!("session/cancel — session={} (notification, no response)", sid);
                }
                // Never answered: session/cancel is a notification.
            }
            _ => {
                if is_notification {
                    eprintln!("ignoring unknown notification: {}", method);
                    continue;
                }
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

#[derive(Clone, Debug)]
struct SessionInfo {
    db_path: PathBuf,
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
    Ok(codetopo_acp::execute_intent(&graph, &intent, Some(db_path)))
}
