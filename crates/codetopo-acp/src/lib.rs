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

/// The `initialize` handshake result (ACP protocol version 1 — an
/// integer on the wire, as the spec requires; an earlier dialect of
/// this agent sent the string "1.0", which strict clients reject).
/// The agent version is the crate version
/// (`env!("CARGO_PKG_VERSION")`) — never a hardcoded string that
/// can drift from the shipped binary.
pub fn initialize_result() -> Value {
    json!({
        "protocolVersion": 1,
        "agentCapabilities": AgentCapabilities::default(),
        "authMethods": [],
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
                    match codetopo_cli::snapshot_db(p, &temp_out, None) {
                        Ok(_) => {
                            let size = std::fs::metadata(&temp_out).map(|m| m.len()).unwrap_or(0);
                            format!("Snapshot written: {} ({} bytes)", temp_out.display(), size)
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
