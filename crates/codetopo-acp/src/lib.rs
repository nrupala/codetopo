// Copyright (C) 2026 Nrupal Akolkar
// SPDX-License-Identifier: AGPL-3.0-or-later
// Part of codetopo — Owned by Nrupal Akolkar · Built with Muse by Meta.

//! Thin adapter: ACP session → codetopo-core / codetopo-store queries.
//! Read-only, deterministic, no duplicated graph logic.
//!
//! Wire protocol: direct newline-delimited JSON-RPC 2.0 over stdio, against
//! the public ACP specification. No SDK, nothing vendored, nothing copied.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::Path;
use codetopo_core::Graph;

pub mod trust;

/// Agent capabilities advertised on initialize, in the ACP wire
/// shape. This agent cannot load sessions, and its prompts are
/// text-only: no image or audio inputs, no embedded context.
#[derive(Serialize, Deserialize, Debug, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct AgentCapabilities {
    pub load_session: bool,
    pub prompt_capabilities: PromptCapabilities,
    pub session_capabilities: SessionCapabilities,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct PromptCapabilities {
    pub image: bool,
    pub audio: bool,
    pub embedded_context: bool,
}

/// No optional session capabilities are offered (no session list,
/// no session fork/resume) — the empty object is the honest answer.
#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct SessionCapabilities {}

/// Whether the client can run Terminal Auth methods. The spec
/// nests the flag at `clientCapabilities.auth.terminal`; some
/// clients (including the ACP Registry's verifier) send a flat
/// `clientCapabilities.terminal` — either is an explicit opt-in.
fn client_supports_terminal(client_params: &Value) -> bool {
    let nested = client_params.pointer("/clientCapabilities/auth/terminal");
    let flat = client_params.pointer("/clientCapabilities/terminal");
    [nested, flat]
        .iter()
        .any(|v| v.and_then(|b| b.as_bool()) == Some(true))
}

/// The `initialize` handshake result (ACP protocol version 1 — an
/// integer on the wire, as the spec requires; an earlier dialect of
/// this agent sent the string "1.0", which strict clients reject).
/// The agent version is the crate version
/// (`env!("CARGO_PKG_VERSION")`) — never a hardcoded string that
/// can drift from the shipped binary.
///
/// `authMethods`: codetopo needs no authentication to use — every
/// session works with zero setup. What it offers, until setup is
/// recorded, is one Terminal Auth method: the proof-verification
/// setup (see [`trust`]), advertised only to clients that declare
/// terminal-auth support, as the spec requires. Once setup is
/// recorded the list is empty again — nothing left to set up.
pub fn initialize_result(client_params: &Value) -> Value {
    initialize_result_in(client_params, &trust::config_dir())
}

/// [`initialize_result`] against an explicit config directory
/// (the seam tests use).
pub fn initialize_result_in(client_params: &Value, config_dir: &Path) -> Value {
    let mut auth_methods = Vec::new();
    if !trust::setup_recorded_in(config_dir) && client_supports_terminal(client_params) {
        auth_methods.push(json!({
            "id": "codetopo-trust-setup",
            "name": "Set up proof verification",
            "description": "codetopo has no account to sign in to and works without setup. This one-time terminal setup provisions the HMAC key that signs codetopo proof certificates, stored only on this machine; keyless setup is offered too.",
            "type": "terminal",
            "args": ["login"],
        }));
    }
    json!({
        "protocolVersion": 1,
        "agentCapabilities": AgentCapabilities::default(),
        "authMethods": auth_methods,
        "agentInfo": {
            "name": "codetopo",
            "title": "codetopo — read-only code-graph agent",
            "version": env!("CARGO_PKG_VERSION"),
        }
    })
}

/// Concatenate the text of a `session/prompt` `prompt` array (ACP
/// content blocks). Non-text blocks contribute nothing: the agent
/// advertises text-only prompt capabilities, so a conforming client
/// never sends anything else.
pub fn prompt_text(blocks: &Value) -> String {
    let mut out = String::new();
    if let Some(arr) = blocks.as_array() {
        for block in arr {
            if block.get("type").and_then(|t| t.as_str()) == Some("text") {
                if let Some(text) = block.get("text").and_then(|t| t.as_str()) {
                    if !out.is_empty() {
                        out.push('\n');
                    }
                    out.push_str(text);
                }
            }
        }
    }
    out
}

/// The `params` of a `session/update` notification carrying one
/// agent message chunk — the only update kind this agent streams.
pub fn message_chunk(session_id: &str, text: &str) -> Value {
    json!({
        "sessionId": session_id,
        "update": {
            "sessionUpdate": "agent_message_chunk",
            "content": {"type": "text", "text": text}
        }
    })
}

/// Intent classification from prompt text (deterministic keyword/regex).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Intent {
    Descendants(String),
    Ancestors(String),
    BlastRadius(String),
    Path(String, String),
    Stats,
    Snapshot,
    Verify,
    Unknown,
    Edit(String), // rename/refactor/fix/etc
}

pub fn classify_intent(prompt: &str) -> Intent {
    let p = prompt.to_lowercase();
    // Edit requests first — read-only refusal.
    if p.contains("rename") || p.contains("refactor") || p.contains("fix ") || p.contains("change ") || p.contains("edit ") || p.contains("rewrite ") || p.contains("delete ") {
        return Intent::Edit(prompt.to_string());
    }
    // Descendants
    if p.contains("descendants of") || p.contains("what does ") && p.contains(" affect") || p.contains("depends on") && p.contains(" what") {
        // Extract rough node id after last of keyword phrase
        let after = p.split("descendants of").nth(1).or_else(|| p.split("what does ").nth(1)).unwrap_or("");
        let nm = after.split_whitespace().next().unwrap_or("unknown").trim();
        return Intent::Descendants(nm.to_string());
    }
    // Ancestors
    if p.contains("ancestors of") || p.contains("what depends on") || p.contains("who calls") || p.contains("parents of") {
        let after = p.split("ancestors of").nth(1).or_else(|| p.split("what depends on").nth(1)).unwrap_or("");
        let nm = after.split_whitespace().next().unwrap_or("unknown").trim();
        return Intent::Ancestors(nm.to_string());
    }
    // Blast radius
    if p.contains("blast radius of") || p.contains("blast-radius of") {
        let after = p.split("blast radius of").nth(1).unwrap_or("");
        let nm = after.split_whitespace().next().unwrap_or("unknown").trim();
        return Intent::BlastRadius(nm.to_string());
    }
    // Path: split on "path from" / " to " to extract node ids from text.
    if p.contains("path from") && p.contains(" to ") {
        let after_from = p.split("path from").nth(1).unwrap_or("");
        let to_parts: Vec<&str> = after_from.split(" to ").collect();
        if to_parts.len() >= 2 {
            let a = to_parts[0].split_whitespace().next().unwrap_or("A");
            let b = to_parts[1].split_whitespace().next().unwrap_or("B");
            return Intent::Path(a.to_string(), b.to_string());
        }
    }
    // Stats
    if p.contains("stats") || p.contains("overview") || p.contains("summarise") || p.contains("summarize") || p.contains("statistics") || p.contains("count") {
        return Intent::Stats;
    }
    // Snapshot
    if p.contains("snapshot") || p.contains("export") && p.contains("json") {
        return Intent::Snapshot;
    }
    // Verify
    if p.contains("verify") || p.contains("audit") || p.contains("check integrity") || p.contains("hmac") {
        return Intent::Verify;
    }
    Intent::Unknown
}

/// Honest refusal for edit/write requests.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct RefusalReply {
    pub refusal: &'static str,
    pub capabilities: Vec<&'static str>,
}

impl RefusalReply {
    pub fn standard() -> Self {
        Self {
            refusal: "I am a read-only code-structure agent; I cannot edit files.",
            capabilities: vec![
                "descendants / ancestors / blast-radius queries",
                "path queries",
                "stats / overview",
                "snapshot export (read-only, temp file)",
                "audit-chain verification",
            ],
        }
    }
}

/// Execute an intent against a loaded graph, returning a result string.
/// The sentence a snapshot answer adds about its proof certificate:
/// the serial and signer fingerprint from the `.proof` sidecar when
/// the snapshot was signed, or the honest note that it was not.
fn snapshot_proof_note(snapshot_out: &Path, key_present: bool) -> String {
    if !key_present {
        return " No proof certificate (no signing key provisioned — run `codetopo-acp login`)."
            .to_string();
    }
    let proof_path = format!("{}.proof", snapshot_out.display());
    match std::fs::read_to_string(&proof_path)
        .ok()
        .and_then(|text| serde_json::from_str::<Value>(&text).ok())
    {
        Some(cert) => format!(
            " Proof certificate {} signed by key {} (HMAC-SHA256, one-year validity).",
            cert["serial"].as_str().unwrap_or("?"),
            cert["signer_key_id"].as_str().unwrap_or("?"),
        ),
        None => " Proof certificate was requested but its sidecar could not be read.".to_string(),
    }
}

pub fn execute_intent(graph: &Graph, intent: &Intent, db_path: Option<&Path>) -> String {
    match intent {
        Intent::Descendants(ref id) => {
            let ids = graph.descendants(id);
            if ids.is_empty() {
                format!("No descendants found for '{}'. (Node may be unknown or leaf.)", id)
            } else {
                format!("Descendants of '{}': {} nodes — {}", id, ids.len(), ids.join(", "))
            }
        }
        Intent::Ancestors(ref id) => {
            let ids = graph.ancestors(id);
            if ids.is_empty() {
                format!("No ancestors found for '{}'.", id)
            } else {
                format!("Ancestors of '{}': {} nodes — {}", id, ids.len(), ids.join(", "))
            }
        }
        Intent::BlastRadius(ref id) => {
            let ids = graph.blast_radius(id);
            if ids.is_empty() {
                format!("Blast radius of '{}': empty (node unknown or isolated).", id)
            } else {
                format!("Blast radius of '{}': {} nodes — {}", id, ids.len(), ids.join(", "))
            }
        }
        Intent::Path(from, to) => {
            match graph.path(from, to) {
                Some(steps) => {
                    let desc = steps.iter().map(|s| format!("{}→{}", s.from, s.to)).collect::<Vec<_>>().join(", ");
                    format!("Path from '{}' to '{}': {} step(s) — {}", from, to, steps.len(), desc)
                }
                None => format!("No path from '{}' to '{}'.", from, to),
            }
        }
        Intent::Stats => {
            format!("Graph stats: {} nodes, {} edges.", graph.node_count(), graph.edge_count())
        }
        Intent::Snapshot => {
            match db_path {
                Some(p) => {
                    let temp_out = std::env::temp_dir().join(format!("codetopo-snap-{}.json", std::process::id()));
                    // The trust setup's key, when provisioned, signs
                    // the snapshot's proof certificate; without one
                    // the snapshot is unsigned, as it always was.
                    let key = trust::hmac_key();
                    match codetopo_cli::snapshot_db(p, &temp_out, key.as_deref()) {
                        Ok(_) => {
                            let size = std::fs::metadata(&temp_out).map(|m| m.len()).unwrap_or(0);
                            let proof_note = snapshot_proof_note(&temp_out, key.is_some());
                            format!("Snapshot written: {} ({} bytes).{}", temp_out.display(), size, proof_note)
                        }
                        Err(e) => format!("Snapshot failed: {}", e),
                    }
                }
                None => "Snapshot requires session db (not loaded).".into(),
            }
        }
        Intent::Verify => {
            match db_path {
                Some(p) => {
                    match codetopo_store::Store::open(p) {
                        Ok(store) => {
                            match store.load_audit() {
                                Ok(audit) => {
                                    if audit.verify() {
                                        "Audit-chain verification: PASS".into()
                                    } else {
                                        "Audit-chain verification: FAIL (chain broken)".into()
                                    }
                                }
                                Err(e) => format!("Audit load failed: {}", e),
                            }
                        }
                        Err(e) => format!("Store open failed: {}", e),
                    }
                }
                None => "Verify requires session db (not loaded).".into(),
            }
        }
        Intent::Edit(_) => RefusalReply::standard().refusal.into(),
        Intent::Unknown => "Unknown intent — please ask about descendants, ancestors, blast-radius, path, stats, snapshot, or verify.".into(),
    }
}

/// Answer produced by routing intent to core/store.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct PromptAnswer {
    pub intent: String,
    pub result: String,
    pub streamed: bool,
}

#[cfg(test)]
mod initialize_tests {
    use super::*;

    fn temp_dir(tag: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!(
            "codetopo-acp-init-{tag}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    const TERMINAL_CAPS: &str = r#"{"clientCapabilities":{"auth":{"terminal":true}}}"#;
    const TERMINAL_CAPS_FLAT: &str = r#"{"clientCapabilities":{"terminal":true}}"#;
    const NO_CAPS: &str = r#"{"clientCapabilities":{}}"#;

    #[test]
    fn terminal_method_offered_before_setup_to_terminal_clients() {
        let dir = temp_dir("offered");
        for caps in [TERMINAL_CAPS, TERMINAL_CAPS_FLAT] {
            let params: Value = serde_json::from_str(caps).expect("caps json");
            let result = initialize_result_in(&params, &dir);
            let methods = result["authMethods"].as_array().expect("authMethods array");
            assert_eq!(methods.len(), 1, "caps: {caps}");
            assert_eq!(methods[0]["type"], "terminal");
            assert_eq!(methods[0]["id"], "codetopo-trust-setup");
            assert_eq!(methods[0]["args"], json!(["login"]));
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn no_method_without_terminal_capability() {
        // The spec: terminal methods are advertised only when the
        // client opted in. A bare initialize gets an empty list —
        // and loses nothing, since no setup is required to use the
        // agent at all.
        let dir = temp_dir("nocaps");
        let params: Value = serde_json::from_str(NO_CAPS).expect("caps json");
        let result = initialize_result_in(&params, &dir);
        assert_eq!(result["authMethods"], json!([]));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn no_method_after_setup_is_recorded() {
        let dir = temp_dir("recorded");
        trust::record_setup_in(&dir, None).expect("record keyless setup");
        let params: Value = serde_json::from_str(TERMINAL_CAPS).expect("caps json");
        let result = initialize_result_in(&params, &dir);
        assert_eq!(result["authMethods"], json!([]));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
