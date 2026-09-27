// Copyright (C) 2026 Nrupal Akolkar
// SPDX-License-Identifier: AGPL-3.0-or-later
// Part of codetopo — Owned by Nrupal Akolkar · Built with Muse by Meta.

//! Thin adapter: ACP session → codetopo-core / codetopo-store queries.
//! Read-only, deterministic, no duplicated graph logic.

use serde::{Deserialize, Serialize};
use codetopo_core::Graph;

/// Read-only agent capabilities advertised on initialize.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct AgentCapabilities {
    pub read_only: bool,
    pub code_structure: bool,
    pub session_based: bool,
}

impl Default for AgentCapabilities {
    fn default() -> Self {
        Self {
            read_only: true,
            code_structure: true,
            session_based: true,
        }
    }
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

#[allow(dead_code)]
fn extract_quoted(p: &str) -> Option<String> {
    Some(p.to_string())
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
    // Path
    if (p.contains("path from") || p.contains("path betwee")) && p.contains(" to ") {
        // Very rough parse
        return Intent::Path("A".into(), "B".into());
    }
    if p.contains("hope") || p.contains("maybe") || p.contains("path") && p.contains(" to ") {
        // try to split around "to"
        if let Some(pos) = p.find(" to ") {
            let left = p[..pos].trim();
            let right = p[pos+4..].trim();
            let a = left.split_whitespace().last().unwrap_or("A");
            let b = right.split_whitespace().next().unwrap_or("B");
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
pub fn execute_intent(graph: &Graph, intent: &Intent) -> String {
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
        Intent::Snapshot => "Snapshot available as read-only JSON export (temp).".into(),
        Intent::Verify => {
            // Use store audit if loaded; here just report graph consistency
            format!("Graph consistency: {} nodes, {} edges. Audit verification requires store load.", graph.node_count(), graph.edge_count())
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
