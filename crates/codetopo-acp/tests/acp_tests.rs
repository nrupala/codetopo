// Copyright (C) 2026 Nrupal Akolkar
// SPDX-License-Identifier: AGPL-3.0-or-later
// Part of codetopo — Owned by Nrupal Akolkar · Built with Muse by Meta.

use codetopo_acp::{classify_intent, Intent, RefusalReply, AgentCapabilities};

#[test]
fn intent_descendants() {
    assert_eq!(classify_intent("descendants of foo"), Intent::Descendants("descendants of foo".into()));
}

#[test]
fn intent_ancestors() {
    assert_eq!(classify_intent("ancestors of bar"), Intent::Ancestors("ancestors of bar".into()));
}

#[test]
fn intent_blast_radius() {
    assert_eq!(classify_intent("blast radius of baz"), Intent::BlastRadius("blast radius of baz".into()));
}

#[test]
fn intent_stats() {
    assert_eq!(classify_intent("summarise the codebase"), Intent::Stats);
}

#[test]
fn intent_unknown_is_refused() {
    let r = RefusalReply::standard();
    assert!(r.refusal.contains("read-only"));
}

#[test]
fn capabilities_default_read_only() {
    let c = AgentCapabilities::default();
    assert!(c.read_only);
    assert!(c.code_structure);
}
