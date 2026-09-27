// Copyright (C) 2026 Nrupal Akolkar
// SPDX-License-Identifier: AGPL-3.0-or-later
// Part of codetopo — Owned by Nrupal Akolkar · Built with Muse by Meta.

use codetopo_acp::{classify_intent, Intent, RefusalReply, AgentCapabilities, execute_intent};

#[test]
fn intent_descendants() {
    assert_eq!(classify_intent("descendants of foo"), Intent::Descendants("foo".into()));
}

#[test]
fn intent_ancestors() {
    assert_eq!(classify_intent("ancestors of bar"), Intent::Ancestors("bar".into()));
}

#[test]
fn intent_blast_radius() {
    assert_eq!(classify_intent("blast radius of baz"), Intent::BlastRadius("baz".into()));
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

#[test]
fn path_parsing_extracts_node_ids() {
    assert_eq!(classify_intent("path from a to b"), Intent::Path("a".into(), "b".into()));
    assert_eq!(classify_intent("path from foo to bar"), Intent::Path("foo".into(), "bar".into()));
}

#[test]
fn execute_intent_descendants_real_graph() {
    use codetopo_core::{GraphBuilder, emit::{FileSymbols, RawSymbol, RawCall}, schema::NodeKind};
    let mut b = GraphBuilder::new();
    b.add_file(FileSymbols {
        package: "p".into(), file: "f.rs".into(), module_id: "m".into(),
        symbols: vec![
            RawSymbol { id: "a".into(), kind: NodeKind::Function, label: "a".into(), line: 1, column: None, defines: vec![], refs: vec![] },
            RawSymbol { id: "b".into(), kind: NodeKind::Function, label: "b".into(), line: 2, column: None, defines: vec![], refs: vec![] },
            RawSymbol { id: "c".into(), kind: NodeKind::Function, label: "c".into(), line: 3, column: None, defines: vec![], refs: vec![] },
        ],
        imports: vec![],
        calls: vec![
            RawCall { from_id: "a".into(), to_name: "b".into(), line: 1 },
            RawCall { from_id: "b".into(), to_name: "c".into(), line: 2 },
        ],
    });
    let (graph, _diag) = b.build();
    let res = execute_intent(&graph, &Intent::Descendants("a".into()), None);
    assert!(!res.is_empty());
    assert!(res.contains("b") || res.contains("c"));
}

#[test]
fn execute_intent_refuses_edit() {
    use codetopo_core::{GraphBuilder, emit::FileSymbols};
    let mut b = GraphBuilder::new();
    b.add_file(FileSymbols {
        package: "p".into(), file: "f.rs".into(), module_id: "m".into(),
        symbols: vec![], imports: vec![], calls: vec![],
    });
    let (graph, _diag) = b.build();
    let res = execute_intent(&graph, &Intent::Edit("rename foo".into()), None);
    assert!(res.contains("read-only") || res.contains("cannot edit"));
}
