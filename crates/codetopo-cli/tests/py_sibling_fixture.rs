// Copyright (C) 2026 2351485 Alberta Inc.
// SPDX-License-Identifier: AGPL-3.0-or-later
// Part of codetopo — Owned by 2351485 Alberta Inc. (https://nrupalakolkar.com) · Built with Muse by Meta (https://muse.ai/join).

//! The 0.9.0 hosted test-pass fixture (Aarona's T1/T4 filing): five
//! Python files in one flat package where every module imports a
//! sibling by bare name and every function calls a sibling's function
//! by its module-qualified name (`b.work()`). Ground truth by reading
//! the sources: 7 module-import edges, 7 function-call edges, all
//! internal; the module graph is one cycle through all five modules
//! and the call graph is one cycle through all five functions.
//!
//! Before the fix the indexer reported all 7 imports and all 7 calls
//! as external stubs (only the 7 `reads` edges were internal), and
//! topo-sort over the resulting reads-only set reported a complete
//! order with `cyclic_groups: 0` on a graph whose ground truth is
//! two cycles. These tests pin the corrected behaviour end to end:
//! extraction → build → queries, the same path the hosted service
//! serves.

use std::collections::BTreeSet;
use std::path::PathBuf;

use codetopo_cli::{index_repo, load_graph_from_db, topo_sort_value};
use codetopo_core::{EdgeKind, Graph, NodeKind};

/// Aarona's fixture, verbatim in structure: a imports b, c and calls
/// b.work() and c.work(); b → c; c → d; d → e; e → a, c. Package name
/// at index time is `repo`.
const FILES: [(&str, &str); 5] = [
    ("a.py", "import b\nimport c\n\n\ndef run():\n    b.work()\n    c.work()\n"),
    ("b.py", "import c\n\n\ndef work():\n    c.work()\n"),
    ("c.py", "import d\n\n\ndef work():\n    d.work()\n"),
    ("d.py", "import e\n\n\ndef work():\n    e.work()\n"),
    ("e.py", "import a\nimport c\n\n\ndef work():\n    a.run()\n    c.work()\n"),
];

fn write_fixture(tag: &str) -> PathBuf {
    // One workdir per test: the tests in this file run in parallel
    // threads in one process, so a shared dir would race (one test's
    // cleanup deleting another's fixture mid-write).
    let dir = std::env::temp_dir().join(format!(
        "codetopo-cli-test-{}-py-sibling-fixture-{tag}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    let repo = dir.join("repo");
    std::fs::create_dir_all(&repo).expect("create fixture dir");
    for (name, body) in FILES {
        std::fs::write(repo.join(name), body).expect("write fixture file");
    }
    repo
}

fn indexed_graph(tag: &str) -> Graph {
    let repo = write_fixture(tag);
    let db = repo.parent().expect("workdir").join("fixture.db");
    let report = index_repo(&repo, &db, "repo", None, None).expect("index succeeds");
    assert_eq!(report.files_indexed, 5);
    assert_eq!(report.files_failed, 0);
    load_graph_from_db(&db).expect("load graph")
}

fn pairs(graph: &Graph, kind: EdgeKind) -> BTreeSet<(String, String)> {
    graph
        .edges()
        .filter(|e| e.kind == kind)
        .map(|e| (e.from.clone(), e.to.clone()))
        .collect()
}

fn expected(edges: &[(&str, &str)]) -> BTreeSet<(String, String)> {
    edges.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect()
}

#[test]
fn sibling_imports_and_qualified_calls_are_internal() {
    let graph = indexed_graph("edges");
    // Nothing in the fixture is external: no stub nodes at all.
    let externals: Vec<&str> = graph
        .nodes()
        .filter(|n| n.kind == NodeKind::External)
        .map(|n| n.id.as_str())
        .collect();
    assert!(externals.is_empty(), "external stubs: {externals:?}");

    assert_eq!(
        pairs(&graph, EdgeKind::Imports),
        expected(&[
            ("repo::a", "repo::b"),
            ("repo::a", "repo::c"),
            ("repo::b", "repo::c"),
            ("repo::c", "repo::d"),
            ("repo::d", "repo::e"),
            ("repo::e", "repo::a"),
            ("repo::e", "repo::c"),
        ]),
        "all 7 ground-truth imports, internal"
    );
    assert_eq!(
        pairs(&graph, EdgeKind::Calls),
        expected(&[
            ("repo::a::run", "repo::b::work"),
            ("repo::a::run", "repo::c::work"),
            ("repo::b::work", "repo::c::work"),
            ("repo::c::work", "repo::d::work"),
            ("repo::d::work", "repo::e::work"),
            ("repo::e::work", "repo::a::run"),
            ("repo::e::work", "repo::c::work"),
        ]),
        "all 7 ground-truth calls, internal"
    );
}

#[test]
fn topo_sort_reports_the_fixtures_cycles() {
    let graph = indexed_graph("topo");
    let value = topo_sort_value(&graph);
    // The absence-honesty pin: a cyclic fixture must not come back as
    // a complete acyclic order. The two ground-truth cycles — the
    // module import cycle and the function call cycle — are reported
    // as cyclic groups, and every node is still accounted for.
    assert_eq!(value["cyclic_groups"], 2, "topo-sort value: {value}");
    assert_eq!(value["total_nodes"], 16);
    let cyclic: Vec<&serde_json::Value> = value["groups"]
        .as_array()
        .expect("groups array")
        .iter()
        .filter(|g| g["cyclic"] == true)
        .collect();
    assert_eq!(cyclic.len(), 2);
    let member_sets: BTreeSet<BTreeSet<&str>> = cyclic
        .iter()
        .map(|g| {
            g["members"]
                .as_array()
                .expect("members array")
                .iter()
                .map(|m| m.as_str().expect("member id"))
                .collect()
        })
        .collect();
    let modules: BTreeSet<&str> = ["repo::a", "repo::b", "repo::c", "repo::d", "repo::e"]
        .into_iter()
        .collect();
    let functions: BTreeSet<&str> = [
        "repo::a::run",
        "repo::b::work",
        "repo::c::work",
        "repo::d::work",
        "repo::e::work",
    ]
    .into_iter()
    .collect();
    assert!(member_sets.contains(&modules), "module cycle flagged: {value}");
    assert!(member_sets.contains(&functions), "function cycle flagged: {value}");
}

#[test]
fn blast_radius_counts_the_whole_cycle() {
    let graph = indexed_graph("blast");
    // §6 blast radius counts function dependents (calls + references),
    // transitively: inside the call cycle every function depends on
    // every module's function, so each module's set is all five.
    let all_five: BTreeSet<String> = [
        "repo::a::run",
        "repo::b::work",
        "repo::c::work",
        "repo::d::work",
        "repo::e::work",
    ]
    .into_iter()
    .map(str::to_string)
    .collect();
    for module in ["repo::a", "repo::b", "repo::c", "repo::d", "repo::e"] {
        let got: BTreeSet<String> = graph.blast_radius(module).into_iter().collect();
        assert_eq!(got, all_five, "blast_radius({module})");
    }
}
