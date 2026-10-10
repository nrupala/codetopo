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

use codetopo_cli::{index_repo, load_graph_from_db, restore_snapshot, verify_db, verify_proof_file};
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
    // The proof is a structured certificate, and it verifies against the
    // exact snapshot bytes under the issuing key.
    let proof_text = std::fs::read_to_string(&proof_path).expect("read proof");
    let cert = codetopo_store::ProofCertificate::from_json(&proof_text)
        .expect("proof is a structured certificate");
    assert!(cert.serial.starts_with("CT-"), "certificate carries a serial");
    assert_eq!(cert.issuer, "codetopo");
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    assert_eq!(
        cert.verify(b"test-key", &snap_json, now),
        codetopo_store::CertStatus::Valid,
        "certificate verifies against the snapshot it was issued for"
    );
    verify_proof_file(&snap, b"test-key").expect("verify-proof passes on a fresh snapshot");

    // --- restore into a second db preserves counts ---
    let stats = restore_snapshot(&snap, &db2, Some(b"test-key")).expect("restore succeeds");
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

/// Reindexing a changed tree into the same database must REPLACE the old
/// graph: a renamed symbol's old node may not survive as a stale row.
#[test]
fn reindex_replaces_stale_graph() {
    let workdir = workdir("reindex-replace");
    let repo = fixture_repo(&workdir);
    let db = workdir.join("index.db");

    index_repo(&repo, &db, "fixture", None, None).expect("first index");
    let g1 = load_graph_from_db(&db).expect("load first graph");
    assert!(g1.nodes().any(|n| n.kind == NodeKind::Function && n.label == "b"));

    // Rename fn b -> fn bb (and the call site), then reindex the same db.
    std::fs::write(repo.join("a.rs"), "fn a() {\n    bb();\n}\n").expect("rewrite a.rs");
    std::fs::write(repo.join("b.rs"), "fn bb() {}\n").expect("rewrite b.rs");
    let report = index_repo(&repo, &db, "fixture", None, None).expect("reindex");
    let g2 = load_graph_from_db(&db).expect("load reindexed graph");

    assert!(
        !g2.nodes().any(|n| n.kind == NodeKind::Function && n.label == "b"),
        "stale function node 'b' must not survive reindex"
    );
    assert!(g2.nodes().any(|n| n.kind == NodeKind::Function && n.label == "bb"));

    // Counts must equal a fresh index of the changed tree exactly.
    let fresh_db = workdir.join("fresh.db");
    let fresh = index_repo(&repo, &fresh_db, "fixture", None, None).expect("fresh index");
    assert_eq!(report.nodes, fresh.nodes, "reindex node count == fresh count");
    assert_eq!(report.edges, fresh.edges, "reindex edge count == fresh count");
    assert_eq!(g2.node_count(), fresh.nodes);

    let _ = std::fs::remove_dir_all(&workdir);
}

/// Restore into a populated database is a usage error, not a silent union.
#[test]
fn restore_into_nonempty_db_fails() {
    let workdir = workdir("restore-nonempty");
    let repo = fixture_repo(&workdir);
    let db = workdir.join("index.db");
    let snap = workdir.join("snap.json");

    index_repo(&repo, &db, "fixture", Some(&snap), None).expect("index with snapshot");
    let err = restore_snapshot(&snap, &db, None).expect_err("restore into same db must fail");
    assert!(
        err.to_string().contains("already holds a graph"),
        "error explains the refusal, got: {err}"
    );

    let _ = std::fs::remove_dir_all(&workdir);
}

/// Diagnostics are persisted with the index and replaced on reindex.
#[test]
fn diagnostics_persist_and_clear_on_reindex() {
    use codetopo_store::Store;

    let workdir = workdir("diagnostics-persist");
    let repo = workdir.join("repo");
    std::fs::create_dir_all(&repo).expect("create repo dir");
    // A call to a function that does not exist -> UnresolvedCall diagnostic.
    std::fs::write(repo.join("a.rs"), "fn a() {\n    missing();\n}\n").expect("write a.rs");
    let db = workdir.join("index.db");

    let report = index_repo(&repo, &db, "fixture", None, None).expect("index");
    assert!(report.diagnostics >= 1, "unresolved call is diagnosed");
    let store = Store::open(&db).expect("open store");
    let diags = store.load_diagnostics().expect("load diagnostics");
    assert_eq!(diags.len(), report.diagnostics, "diagnostics persisted in full");
    assert!(
        diags.iter().any(|d| matches!(d, codetopo_core::Diagnostic::UnresolvedCall { .. })),
        "the unresolved call is among them: {diags:?}"
    );
    drop(store);

    // Define the function and reindex: the old diagnostics must be gone.
    std::fs::write(repo.join("b.rs"), "fn missing() {}\n").expect("write b.rs");
    let report2 = index_repo(&repo, &db, "fixture", None, None).expect("reindex");
    assert_eq!(report2.diagnostics, 0, "fixed tree produces no diagnostics");
    let store = Store::open(&db).expect("reopen store");
    assert!(store.load_diagnostics().expect("reload").is_empty());

    let _ = std::fs::remove_dir_all(&workdir);
}

/// A snapshot whose bytes changed after certification must not restore:
/// the certificate binds the exact artifact hash.
#[test]
fn restore_rejects_tampered_snapshot() {
    let workdir = workdir("restore-tampered");
    let repo = fixture_repo(&workdir);
    let db = workdir.join("index.db");
    let snap = workdir.join("snap.json");
    let db2 = workdir.join("restored.db");

    index_repo(&repo, &db, "fixture", Some(&snap), Some(b"test-key")).expect("index");
    let json = std::fs::read_to_string(&snap).expect("read snapshot");
    std::fs::write(&snap, format!("{json}\n")).expect("tamper: trailing newline changes the bytes");

    let err = restore_snapshot(&snap, &db2, Some(b"test-key"))
        .expect_err("tampered snapshot must not restore");
    assert!(
        err.to_string().contains("proof certificate"),
        "error names the failed proof check, got: {err}"
    );

    let _ = std::fs::remove_dir_all(&workdir);
}

/// An expired certificate no longer attests: restore must refuse it.
#[test]
fn restore_rejects_expired_certificate() {
    let workdir = workdir("restore-expired");
    let repo = fixture_repo(&workdir);
    let db = workdir.join("index.db");
    let snap = workdir.join("snap.json");
    let db2 = workdir.join("restored.db");

    index_repo(&repo, &db, "fixture", Some(&snap), Some(b"test-key")).expect("index");
    let json = std::fs::read_to_string(&snap).expect("read snapshot");
    let parsed = codetopo_core::Snapshot::from_json(&json).expect("parse snapshot");
    // Re-issue the certificate as of the epoch: long past its window.
    let expired = codetopo_store::issue_certificate(b"test-key", &json, &parsed.audit_head, 0);
    std::fs::write(format!("{}.proof", snap.display()), expired.to_json())
        .expect("write expired cert");

    let err = restore_snapshot(&snap, &db2, Some(b"test-key"))
        .expect_err("expired certificate must not restore");
    assert!(
        err.to_string().contains("expired"),
        "error names the expiry, got: {err}"
    );

    let _ = std::fs::remove_dir_all(&workdir);
}

/// A snapshot from an unsupported schema major is rejected loudly.
#[test]
fn restore_rejects_unknown_schema_major() {
    let workdir = workdir("restore-schema");
    let repo = fixture_repo(&workdir);
    let db = workdir.join("index.db");
    let snap = workdir.join("snap.json");
    let db2 = workdir.join("restored.db");

    index_repo(&repo, &db, "fixture", Some(&snap), None).expect("index without proof");
    let json = std::fs::read_to_string(&snap).expect("read snapshot");
    std::fs::write(&snap, json.replace("1.0-draft", "2.0")).expect("rewrite schema version");

    let err = restore_snapshot(&snap, &db2, None).expect_err("schema major 2 must be rejected");
    assert!(
        err.to_string().contains("schema_version"),
        "error names the schema version, got: {err}"
    );

    let _ = std::fs::remove_dir_all(&workdir);
}

/// A snapshot whose embedded audit chain was altered must not verify.
#[test]
fn verify_proof_rejects_altered_audit_chain() {
    let workdir = workdir("verify-altered-chain");
    let repo = fixture_repo(&workdir);
    let db = workdir.join("index.db");
    let snap = workdir.join("snap.json");

    index_repo(&repo, &db, "fixture", Some(&snap), Some(b"test-key")).expect("index");
    let json = std::fs::read_to_string(&snap).expect("read snapshot");
    std::fs::write(&snap, json.replace("index_started", "index_hacked"))
        .expect("alter one audit entry's op");

    assert!(
        verify_proof_file(&snap, b"test-key").is_err(),
        "an altered audit chain must fail verification"
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
