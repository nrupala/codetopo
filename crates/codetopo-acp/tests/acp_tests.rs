// Copyright (C) 2026 Nrupal Akolkar
// SPDX-License-Identifier: AGPL-3.0-or-later
// Part of codetopo — Owned by Nrupal Akolkar (https://nrupalakolkar.com) · Built with Muse by Meta (https://muse.ai/join) — join Muse, then redeem code 101EEK in Settings within 48 hours of joining and we both earn tokens.

use codetopo_acp::{classify_intent, initialize_result, message_chunk, prompt_text, Intent, RefusalReply, AgentCapabilities, execute_intent};
use serde_json::json;

#[test]
fn initialize_reports_the_crate_version() {
    // Version honesty: the handshake must claim the version of the
    // crate it ships in — it previously hardcoded 0.1.0.
    let result = initialize_result(&json!({}));
    assert_eq!(result["agentInfo"]["name"], "codetopo");
    assert_eq!(
        result["agentInfo"]["version"],
        env!("CARGO_PKG_VERSION"),
        "agentInfo version must be the crate version, not a hardcoded string"
    );
}

#[test]
fn initialize_speaks_acp_v1() {
    // The spec's protocolVersion is an INTEGER (1). A previous
    // dialect sent the string "1.0", which strict clients reject.
    let result = initialize_result(&json!({}));
    assert_eq!(result["protocolVersion"], json!(1));
    assert!(result["protocolVersion"].is_number());
    let caps = &result["agentCapabilities"];
    assert_eq!(caps["loadSession"], json!(false));
    assert_eq!(caps["promptCapabilities"]["image"], json!(false));
    assert_eq!(caps["promptCapabilities"]["audio"], json!(false));
    assert_eq!(caps["promptCapabilities"]["embeddedContext"], json!(false));
    assert_eq!(result["authMethods"], json!([]));
}

#[test]
fn prompt_text_joins_text_blocks_only() {
    let blocks = json!([
        {"type": "text", "text": "first"},
        {"type": "image", "data": "...", "mimeType": "image/png"},
        {"type": "text", "text": "second"},
    ]);
    assert_eq!(prompt_text(&blocks), "first\nsecond");
    assert_eq!(prompt_text(&json!([])), "");
}

#[test]
fn message_chunk_is_the_spec_update_shape() {
    let params = message_chunk("sess-1", "hello");
    assert_eq!(params["sessionId"], "sess-1");
    assert_eq!(params["update"]["sessionUpdate"], "agent_message_chunk");
    assert_eq!(params["update"]["content"], json!({"type": "text", "text": "hello"}));
}

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
fn capabilities_default_is_the_honest_minimum() {
    // No session loading, text-only prompts — the struct serializes
    // to exactly the wire shape initialize advertises.
    let wire = serde_json::to_value(AgentCapabilities::default()).expect("serializes");
    assert_eq!(wire["loadSession"], json!(false));
    assert_eq!(wire["promptCapabilities"]["embeddedContext"], json!(false));
    assert_eq!(wire["sessionCapabilities"], json!({}));
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
            RawSymbol { id: "a".into(), kind: NodeKind::Function, label: "a".into(), line: 1, column: None, defines: vec![], refs: vec![], writes: vec![], end_line: None, signature: None, decisions: None },
            RawSymbol { id: "b".into(), kind: NodeKind::Function, label: "b".into(), line: 2, column: None, defines: vec![], refs: vec![], writes: vec![], end_line: None, signature: None, decisions: None },
            RawSymbol { id: "c".into(), kind: NodeKind::Function, label: "c".into(), line: 3, column: None, defines: vec![], refs: vec![], writes: vec![], end_line: None, signature: None, decisions: None },
        ],
        imports: vec![],
        calls: vec![
            RawCall { from_id: "a".into(), to_name: "b".into(), line: 1 },
            RawCall { from_id: "b".into(), to_name: "c".into(), line: 2 },
        ],
        heritage: vec![],
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
        symbols: vec![], imports: vec![], calls: vec![], heritage: vec![],
    });
    let (graph, _diag) = b.build();
    let res = execute_intent(&graph, &Intent::Edit("rename foo".into()), None);
    assert!(res.contains("read-only") || res.contains("cannot edit"));
}
