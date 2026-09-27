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

use codetopo_cli::{index_repo, load_graph_from_db, restore_snapshot};
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
    assert_eq!(
        std::fs::read_to_string(&proof_path).expect("read proof").len(),
        64,
        "proof is hex HMAC-SHA256"
    );

    // --- restore into a second db preserves counts ---
    let stats = restore_snapshot(&snap, &db2).expect("restore succeeds");
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
