// Copyright (C) 2026 Nrupal Akolkar
// SPDX-License-Identifier: AGPL-3.0-or-later
// Part of codetopo — Owned by Nrupal Akolkar · Built with Muse by Meta.

//! Thin adapter: ACP session → codetopo-core / codetopo-store queries.
//! Read-only, deterministic, no duplicated graph logic.

use serde::{Deserialize, Serialize};

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
}

/// Route a prompt to an intent.
pub fn classify_intent(prompt: &str) -> Intent {
    let p = prompt.to_lowercase();
    if p.contains("descendants of") || p.contains("what does ") && p.contains(" affect") {
        // Extract node name roughly after keyword
        return Intent::Descendants(prompt.to_string());
    }
    if p.contains("ancestors of") || p.contains("what depends on") {
        return Intent::Ancestors(prompt.to_string());
    }
    if p.contains("blast radius of") {
        return Intent::BlastRadius(prompt.to_string());
    }
    if p.contains("path from") && p.contains(" to ") {
        return Intent::Path("A".into(), "B".into()); // real extraction would parse tokens
    }
    if p.contains("stats") || p.contains("overview") || p.contains("summarise") || p.contains("summarize") {
        return Intent::Stats;
    }
    if p.contains("snapshot") {
        return Intent::Snapshot;
    }
    if p.contains("verify") {
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

/// Answer produced by routing intent to core/store.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct PromptAnswer {
    pub intent: String,
    pub result: String,
    pub streamed: bool,
}
