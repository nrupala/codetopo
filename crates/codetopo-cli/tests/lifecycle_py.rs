// Copyright (C) 2026 2351485 Alberta Inc.
// SPDX-License-Identifier: AGPL-3.0-or-later
// Part of codetopo — Owned by 2351485 ALBERTA INC. (https://nrupalakolkar.com) · Built with Muse by Meta (https://muse.ai/join).

//! F5 — the lifecycle walk on a copy of the Python known-answer
//! fixture (`codetopo-extract/tests/fixture-py`, package `quarry`):
//!
//! 1. index → snapshot; a second index of the untouched tree
//!    produces an identical snapshot — byte-identical in everything
//!    except the audit log's wall-clock timestamps and the hashes
//!    chained from them (the one place a snapshot honestly records
//!    *when* it was made; see [`assert_snapshots_match`] for how the
//!    comparison keeps its teeth there);
//! 2. snapshot → restore → the restored graph is the same graph
//!    (same node ids, same edge triples);
//! 3. a self-diff is empty;
//! 4. editing exactly one function (renaming `Circle.area` to
//!    `Circle.surface_area` in the copy) and re-indexing yields a
//!    diff that names exactly that function — one removal, one
//!    addition, no changed nodes — with the blast-radius impact
//!    confined to those two ids.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use codetopo_cli::{index_repo, load_graph_from_db, restore_snapshot};
use codetopo_core::measure::diff_graphs;
use codetopo_core::{Graph, Snapshot};
use codetopo_store::AuditLog;

fn fixture_py_source() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../codetopo-extract/tests/fixture-py")
}

/// Copy the fixture into a fresh scratch dir (the edit step must
/// never touch the committed fixture). The cli test suite's own
/// workdir idiom: <temp>/codetopo-cli-test-<pid>-<name>/.
fn copy_fixture() -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "codetopo-cli-test-{}-lifecycle-py",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create workdir");
    let repo = dir.join("repo");
    std::fs::create_dir_all(&repo).expect("create repo dir");
    let src = fixture_py_source();
    for name in ["pyproject.toml", "__init__.py", "core.py", "shapes.py", "README.md"] {
        std::fs::copy(src.join(name), repo.join(name)).expect("copy fixture file");
    }
    repo
}

/// Assert two snapshots of the same tree are the same snapshot.
///
/// Everything must match exactly — schema, nodes, edges, and the
/// audit log's recorded operations — with one honest exception: the
/// audit entries' `ts` fields are wall-clock seconds
/// (`codetopo_store`'s `now_ts`), and each entry's `hash`, the
/// `prev_hash` links, and the snapshot's `audit_head` are chained
/// from those timestamps, so two runs that straddle a one-second
/// boundary differ in exactly those fields. (That was the flake
/// this replaces: a raw byte comparison failed ~1–2% of runs,
/// whenever the two index runs crossed a second.)
///
/// The exception is not a blind spot. Before anything is
/// normalized, each snapshot must prove its own chain: the entries
/// recompute and verify end-to-end for *their* timestamps
/// ([`AuditLog::verify`]), the links and the head line up, the
/// timestamps parse as seconds and never run backwards, and the
/// two logs record the same operations (`seq`/`op`/`payload`
/// identical, entry for entry). Only the timestamp-derived fields
/// are then blanked, and the normalized snapshots must be equal
/// *and* serialize to identical bytes — the byte-identity claim
/// survives everywhere it was ever true.
fn assert_snapshots_match(bytes1: &[u8], bytes2: &[u8]) {
    let snap1 = Snapshot::from_json(std::str::from_utf8(bytes1).expect("snapshot 1 is UTF-8"))
        .expect("snapshot 1 parses");
    let snap2 = Snapshot::from_json(std::str::from_utf8(bytes2).expect("snapshot 2 is UTF-8"))
        .expect("snapshot 2 parses");

    for (label, snap) in [("first", &snap1), ("second", &snap2)] {
        let log = AuditLog::from_snapshot_entries(&snap.audit_log);
        assert!(
            log.verify(),
            "{label} snapshot's audit chain recomputes for its own timestamps"
        );
        assert_eq!(
            snap.audit_head,
            snap.audit_log.last().map(|e| e.hash.clone()).unwrap_or_default(),
            "{label} snapshot's head is its chain's last link"
        );
        let mut prev_ts: Option<u64> = None;
        for (i, entry) in snap.audit_log.iter().enumerate() {
            assert_eq!(entry.seq, i as u64, "{label} audit seq is positional");
            assert_eq!(
                entry.prev_hash,
                if i == 0 {
                    "GENESIS".to_string()
                } else {
                    snap.audit_log[i - 1].hash.clone()
                },
                "{label} audit entry {i} links to its predecessor"
            );
            let ts: u64 = entry
                .ts
                .parse()
                .unwrap_or_else(|_| panic!("{label} audit entry {i} ts is seconds: {}", entry.ts));
            if let Some(prev) = prev_ts {
                assert!(ts >= prev, "{label} audit timestamps never run backwards");
            }
            prev_ts = Some(ts);
        }
    }

    assert_eq!(
        snap1.audit_log.len(),
        snap2.audit_log.len(),
        "both runs record the same number of audit operations"
    );
    for (e1, e2) in snap1.audit_log.iter().zip(&snap2.audit_log) {
        assert_eq!(
            (e1.seq, &e1.op, &e1.payload),
            (e2.seq, &e2.op, &e2.payload),
            "both runs record the same audit operations"
        );
    }

    let normalize = |snap: &Snapshot| {
        let mut snap = snap.clone();
        snap.audit_head.clear();
        for entry in &mut snap.audit_log {
            entry.ts.clear();
            entry.prev_hash.clear();
            entry.hash.clear();
        }
        snap
    };
    let (norm1, norm2) = (normalize(&snap1), normalize(&snap2));
    assert_eq!(
        norm1, norm2,
        "snapshots of an unchanged tree agree outside the audit clock"
    );
    assert_eq!(
        norm1.to_json_pretty().expect("normalized 1 serializes"),
        norm2.to_json_pretty().expect("normalized 2 serializes"),
        "normalized snapshots are byte-identical"
    );
}

fn node_ids(graph: &Graph) -> BTreeSet<String> {
    graph.nodes().map(|n| n.id.clone()).collect()
}

fn edge_triples(graph: &Graph) -> BTreeSet<(String, String, String)> {
    graph
        .edges()
        .map(|e| (e.from.clone(), format!("{:?}", e.kind), e.to.clone()))
        .collect()
}

#[test]
fn py_snapshot_restore_and_edit_diff() {
    let repo = copy_fixture();
    let work = repo.parent().expect("workdir").to_path_buf();
    let db1 = work.join("one.db");
    let db2 = work.join("restored.db");
    let db3 = work.join("two.db");
    let db4 = work.join("edited.db");
    let snap1 = work.join("snap1.json");
    let snap2 = work.join("snap2.json");

    // 1. Index with a snapshot; index again — the same snapshot
    // (byte-identical outside the audit log's wall-clock fields;
    // see assert_snapshots_match).
    let report = index_repo(&repo, &db1, "quarry", Some(&snap1), Some(b"test-key"))
        .expect("index succeeds");
    assert_eq!(report.files_indexed, 4, "3 .py files + the README");
    assert_eq!(report.files_failed, 0);
    assert_eq!(report.nodes, 29, "fixture README shows the arithmetic");
    assert_eq!(report.edges, 36);
    let report2 = index_repo(&repo, &db3, "quarry", Some(&snap2), Some(b"test-key"))
        .expect("second index succeeds");
    assert_eq!(report2.nodes, report.nodes);
    let bytes1 = std::fs::read(&snap1).expect("read snapshot 1");
    let bytes2 = std::fs::read(&snap2).expect("read snapshot 2");
    assert_snapshots_match(&bytes1, &bytes2);
    let snap_value: serde_json::Value =
        serde_json::from_slice(&bytes1).expect("snapshot is JSON");
    assert_eq!(
        snap_value["schema_version"],
        serde_json::json!("2.0"),
        "the snapshot carries core schema 2.0 (the CODE profile's dictionary)"
    );

    // 2. Restore: the same graph comes back.
    let g1 = load_graph_from_db(&db1).expect("load original");
    let stats = restore_snapshot(&snap1, &db2, Some(b"test-key"), false).expect("restore succeeds");
    let g2 = load_graph_from_db(&db2).expect("load restored");
    assert_eq!(stats.nodes, g1.node_count());
    assert_eq!(node_ids(&g2), node_ids(&g1), "restore preserves every node id");
    assert_eq!(edge_triples(&g2), edge_triples(&g1), "restore preserves every edge");

    // 3. A self-diff is empty in every component.
    let g1_again = load_graph_from_db(&db1).expect("reload original");
    let self_diff = diff_graphs(&g1, &g1_again);
    assert!(self_diff.added_nodes.is_empty());
    assert!(self_diff.removed_nodes.is_empty());
    assert!(self_diff.changed_nodes.is_empty());
    assert!(self_diff.added_edges.is_empty());
    assert!(self_diff.removed_edges.is_empty());

    // 4. Rename exactly one function in the copy and re-index.
    let shapes_path = repo.join("shapes.py");
    let shapes = std::fs::read_to_string(&shapes_path).expect("read shapes.py");
    let edited = shapes.replace(
        "    def area(self):\n        return math.pi",
        "    def surface_area(self):\n        return math.pi",
    );
    assert_ne!(edited, shapes, "the edit actually landed");
    std::fs::write(&shapes_path, edited).expect("write edited shapes.py");
    index_repo(&repo, &db4, "quarry", None, None).expect("re-index succeeds");
    let g4 = load_graph_from_db(&db4).expect("load edited graph");

    let diff = diff_graphs(&g1, &g4);
    let removed: Vec<&str> = diff.removed_nodes.iter().map(|n| n.id.as_str()).collect();
    let added: Vec<&str> = diff.added_nodes.iter().map(|n| n.id.as_str()).collect();
    assert_eq!(removed, vec!["quarry::shapes::Circle::area"]);
    assert_eq!(added, vec!["quarry::shapes::Circle::surface_area"]);
    assert!(
        diff.changed_nodes.is_empty(),
        "nothing else changed: {:?}",
        diff.changed_nodes
    );
    // The blast-radius movement is confined to the renamed pair.
    assert!(!diff.impact.is_empty(), "the rename moves blast radius");
    for entry in &diff.impact {
        assert!(
            entry.id == "quarry::shapes::Circle::area"
                || entry.id == "quarry::shapes::Circle::surface_area",
            "impact names only the renamed function, got {}",
            entry.id
        );
    }
}
