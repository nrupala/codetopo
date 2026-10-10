// Copyright (C) 2026 Nrupal Akolkar
// SPDX-License-Identifier: AGPL-3.0-or-later
// Part of codetopo — Owned by Nrupal Akolkar · Built with Muse by Meta.

//! Integration test for the `codetopo` CLI (worker W5).
//!
//! Builds a tiny 3-file fixture repo (a.rs: `fn a` calls `b`; b.rs: `fn b`;
//! c.ts: a class), drives the library's `index_repo` directly, then asserts
//! the aggregate counts, a `descendants` query, the snapshot (+ proof) files,
//! and a restore round-trip into a second database. A second test drives the
//! built binary to check the exit-code-2 contract for unknown ids.

use std::path::{Path, PathBuf};
use std::process::Command;

use codetopo_cli::{index_repo, load_graph_from_db, restore_snapshot, verify_db};
use codetopo_core::{EdgeKind, NodeKind};

/// Fresh scratch dir per test: <temp>/codetopo-cli-test-<pid>-<name>/.
/// The per-test `name` keeps parallel tests from sharing (and deleting)
/// each other's directories.
fn workdir(name: &str) -> PathBuf {
    let dir =
        std::env::temp_dir().join(format!("codetopo-cli-test-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create workdir");
    dir
}

/// Fixture repo: a.rs (`fn a` → calls `b`), b.rs (`fn b`), c.ts (a class).
fn fixture_repo(workdir: &Path) -> PathBuf {
    let repo = workdir.join("repo");
    std::fs::create_dir_all(&repo).expect("create repo dir");
    std::fs::write(repo.join("a.rs"), "fn a() {\n    b();\n}\n").expect("write a.rs");
    std::fs::write(repo.join("b.rs"), "fn b() {}\n").expect("write b.rs");
    std::fs::write(repo.join("c.ts"), "class C {\n}\n").expect("write c.ts");
    repo
}

/// Find a symbol node id by (kind, label) — robust to the extractor's exact
/// id scheme (`{package}::{module}::{name}`).
fn find_id(graph: &codetopo_core::Graph, kind: NodeKind, label: &str) -> String {
    graph
        .nodes()
        .find(|n| n.kind == kind && n.label == label)
        .unwrap_or_else(|| panic!("expected a {kind:?} node labeled '{label}'"))
        .id
        .clone()
}

#[test]
fn index_query_snapshot_restore_round_trip() {
    let workdir = workdir("round-trip");
    let repo = fixture_repo(&workdir);
    let db = workdir.join("index.db");
    let snap = workdir.join("snap.json");
    let db2 = workdir.join("restored.db");

    // --- index (as a library call) ---
    let report = index_repo(&repo, &db, "fixture", Some(&snap), Some(b"test-key"))
        .expect("index_repo succeeds");
    assert_eq!(report.files_indexed, 3, "all three fixture files indexed");
    assert_eq!(report.files_failed, 0, "no fixture file may fail extraction");
    // 1 package + 3 modules + 3 files + 3 symbols (a, b, C) = 10 nodes;
    // 9 contains edges + 1 calls edge (a -> b) = 10 edges. Lower bounds: the
    // extractor may emit more, never less.
    assert!(report.nodes >= 10, "nodes >= 10, got {}", report.nodes);
    assert!(report.edges >= 10, "edges >= 10, got {}", report.edges);
    assert!(db.exists(), "database file created");

    // --- query: descendants("...::a") contains b's id ---
    let graph = load_graph_from_db(&db).expect("load graph from db");
    let a_id = find_id(&graph, NodeKind::Function, "a");
    let b_fn_id = find_id(&graph, NodeKind::Function, "b");
    let _c_id = find_id(&graph, NodeKind::Class, "C");
    // NOTE (frozen cross-crate semantics): W3 names b.rs's module
    // `fixture::b` and its function `fixture::b::b`. Core resolves the call
    // target `b` by `::b` suffix within the same-package tier, picking the
    // lexicographically first candidate — the *module* node `fixture::b`,
    // not the function. So "b's id" here is the id the calls edge actually
    // resolved to; the test asserts the descendants query is consistent
    // with the stored calls edge rather than hardcoding that resolution.
    // Whether resolve should prefer symbol nodes over module nodes is a
    // core/extract design question — flagged in the W5 report.
    let call_target = graph
        .edges()
        .find(|e| e.kind == EdgeKind::Calls && e.from == a_id)
        .map(|e| e.to.clone())
        .expect("a calls something");
    assert!(
        call_target.ends_with("::b") || call_target == "fixture::b",
        "call target should be b-related, got {call_target}"
    );
    let desc = graph.descendants(&a_id);
    assert!(
        desc.contains(&call_target),
        "descendants({a_id}) must contain b's resolved id {call_target}; got {desc:?}"
    );
    assert!(graph.get(&b_fn_id).is_some(), "function b is indexed");
    // ancestors is the reverse view.
    assert!(graph.ancestors(&call_target).contains(&a_id));

    // --- snapshot file (+ proof, since a key was given) ---
    assert!(snap.exists(), "snapshot file written");
    let snap_json = std::fs::read_to_string(&snap).expect("read snapshot");
    assert!(snap_json.contains("fixture"), "snapshot mentions the package");
    let proof_path = format!("{}.proof", snap.display());
    assert!(
        PathBuf::from(&proof_path).exists(),
        "proof file written alongside the snapshot"
    );
    let cert: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&proof_path).expect("read proof"))
            .expect("proof is a JSON certificate");
    assert_eq!(cert["certificate"], "codetopo-snapshot-proof");
    assert_eq!(cert["algorithm"], "HMAC-SHA256");
    assert_eq!(cert["snapshot_sha256"].as_str().unwrap().len(), 64);
    assert_eq!(cert["signature"].as_str().unwrap().len(), 64);

    // --- restore into a second db preserves counts ---
    let stats = restore_snapshot(&snap, &db2, Some(b"test-key"), false).expect("restore succeeds");
    let g1 = load_graph_from_db(&db).expect("reload db1");
    let g2 = load_graph_from_db(&db2).expect("reload db2");
    assert_eq!(g2.node_count(), g1.node_count(), "restore preserves node count");
    assert_eq!(g2.edge_count(), g1.edge_count(), "restore preserves edge count");
    assert_eq!(stats.nodes, g1.node_count());
    assert_eq!(stats.edges, g1.edge_count());
    // Queries work on the restored graph too.
    let a2 = find_id(&g2, NodeKind::Function, "a");
    let target2 = g2
        .edges()
        .find(|e| e.kind == EdgeKind::Calls && e.from == a2)
        .map(|e| e.to.clone())
        .expect("restored graph keeps the calls edge");
    assert!(g2.descendants(&a2).contains(&target2));

    let _ = std::fs::remove_dir_all(&workdir);
}

#[test]
fn unknown_id_exits_2() {
    let workdir = workdir("exit-code");
    let repo = fixture_repo(&workdir);
    let db = workdir.join("index.db");
    index_repo(&repo, &db, "fixture", None, None).expect("index_repo succeeds");

    let bin = env!("CARGO_BIN_EXE_codetopo");
    let status = Command::new(bin)
        .args(["descendants", "no::such::id", "--db"])
        .arg(&db)
        .status()
        .expect("run codetopo binary");
    assert_eq!(
        status.code(),
        Some(2),
        "unknown id must exit with code 2"
    );

    let _ = std::fs::remove_dir_all(&workdir);
}

/// `verify` must FAIL (Err -> non-zero exit) when the stored audit chain
/// has been tampered with — printing "chain valid: false" and exiting 0
/// would let a broken chain pass every script that checks the exit code.
#[test]
fn verify_db_fails_on_tampered_chain() {
    use codetopo_store::{AuditLog, Store};

    let workdir = workdir("verify-tamper");
    let repo = fixture_repo(&workdir);
    let db = workdir.join("index.db");
    index_repo(&repo, &db, "fixture", None, None).expect("index");

    // Untouched chain verifies.
    verify_db(&db).expect("fresh chain verifies");

    // Tamper: rewrite one entry's payload without recomputing its hash.
    let mut store = Store::open(&db).expect("open store");
    let audit = store.load_audit().expect("load audit");
    let mut entries = audit.entries().to_vec();
    assert!(entries.len() > 1, "fixture index writes multiple entries");
    entries[1].payload = "tampered".to_string();
    store
        .save_audit(&AuditLog::from_entries(entries))
        .expect("save tampered audit");
    drop(store);

    assert!(
        verify_db(&db).is_err(),
        "verify_db must error on a tampered chain"
    );
}

/// Reindexing the same database with a different repo must REPLACE the
/// graph, not union into it — a stale symbol from the first repo has
/// no business surviving the second index.
#[test]
fn reindex_replaces_not_unions() {
    let workdir = workdir("reindex-replaces");
    let repo = fixture_repo(&workdir);
    let db = workdir.join("index.db");
    index_repo(&repo, &db, "fixture", None, None).expect("first index");
    let g1 = load_graph_from_db(&db).expect("load first graph");
    assert!(g1.nodes().any(|n| n.label == "a"));

    // Second repo: only b.rs (no a, no c.ts).
    let repo2 = workdir.join("repo2");
    std::fs::create_dir_all(&repo2).expect("create repo2");
    std::fs::write(repo2.join("b.rs"), "fn b() {}\n").expect("write b.rs");
    index_repo(&repo2, &db, "fixture", None, None).expect("second index");
    let g2 = load_graph_from_db(&db).expect("load second graph");
    assert!(
        !g2.nodes().any(|n| n.label == "a"),
        "stale symbol 'a' survived a reindex (union semantics)"
    );
    assert!(g2.nodes().any(|n| n.label == "b"));
}

/// Restore verifies before it writes: wrong key, a tampered declared
/// head, and a missing key are all refused; only the right key (or, for
/// the chain alone, the explicit unverified flag) restores.
#[test]
fn restore_verification_gates() {
    let workdir = workdir("restore-gates");
    let repo = fixture_repo(&workdir);
    let db = workdir.join("index.db");
    let snap = workdir.join("snap.json");
    index_repo(&repo, &db, "fixture", Some(&snap), Some(b"right-key")).expect("index");

    // Proof exists, no key, no flag -> refused.
    let db_a = workdir.join("a.db");
    assert!(restore_snapshot(&snap, &db_a, None, false).is_err());
    // Wrong key -> refused.
    let db_b = workdir.join("b.db");
    assert!(restore_snapshot(&snap, &db_b, Some(b"wrong-key"), false).is_err());
    // Right key -> restores.
    let db_c = workdir.join("c.db");
    assert!(restore_snapshot(&snap, &db_c, Some(b"right-key"), false).is_ok());

    // Tamper with the declared audit head: the chain gate must refuse
    // even on the unverified path (the chain needs no key to check).
    let text = std::fs::read_to_string(&snap).expect("read snapshot");
    let head = codetopo_core::Snapshot::from_json(&text).unwrap().audit_head;
    let tampered = text.replacen(
        &format!("\"audit_head\": \"{head}\""),
        "\"audit_head\": \"0000000000000000000000000000000000000000000000000000000000000000\"",
        1,
    );
    assert_ne!(tampered, text, "tamper actually changed the snapshot");
    let snap2 = workdir.join("snap-tampered.json");
    std::fs::write(&snap2, &tampered).expect("write tampered snapshot");
    // The tampered file has no matching proof next to it either; the
    // chain gate fires first, so both paths must refuse.
    let db_d = workdir.join("d.db");
    assert!(restore_snapshot(&snap2, &db_d, Some(b"right-key"), false).is_err());
    let db_e = workdir.join("e.db");
    assert!(restore_snapshot(&snap2, &db_e, None, true).is_err());
}
