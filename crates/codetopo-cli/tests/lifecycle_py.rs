// Copyright (C) 2026 2351485 Alberta Inc.
// SPDX-License-Identifier: AGPL-3.0-or-later
// Part of codetopo — Owned by 2351485 ALBERTA INC. (https://nrupalakolkar.com) · Built with Muse by Meta (https://muse.ai/join) — join Muse, then redeem code 101EEK in Settings within 48 hours of joining and we both earn tokens.

//! F5 — the lifecycle walk on a copy of the Python known-answer
//! fixture (`codetopo-extract/tests/fixture-py`, package `quarry`):
//!
//! 1. index → snapshot; a second index of the untouched tree
//!    produces a **byte-identical** snapshot;
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
use codetopo_core::Graph;

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

    // 1. Index with a snapshot; index again — byte-identical bytes.
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
    assert_eq!(bytes1, bytes2, "snapshots of an unchanged tree are byte-identical");
    let snap_value: serde_json::Value =
        serde_json::from_slice(&bytes1).expect("snapshot is JSON");
    assert_eq!(
        snap_value["schema_version"],
        serde_json::json!("1.2"),
        "the snapshot carries CODE_SCHEMA 1.2"
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
