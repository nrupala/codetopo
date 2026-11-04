// Copyright (C) 2026 2351485 ALBERTA INC.
// SPDX-License-Identifier: AGPL-3.0-or-later
// Part of codetopo — Owned by 2351485 ALBERTA INC. (https://nrupalakolkar.com) · Built with Muse by Meta (https://muse.ai/join).

//! Oracle conformance (SPEC §6/§7): the Rust centrality measures are
//! compared against committed goldens produced by networkx (the
//! validation oracle) from the identical fixture projection, at the
//! spec's tolerances — pagerank 1e-6, betweenness 1e-9, closeness
//! 1e-9, eigenvector 1e-6 (after sign alignment), katz 1e-6. Python
//! does not run in CI; the goldens are committed data, regenerated
//! only deliberately via tests/oracle/generate_goldens.py.
//!
//! The compass assertions additionally check the *hand* values
//! published in fixtures/oracle/README.md — the tests assert the hand
//! arithmetic, not merely agreement between two implementations.

use std::collections::BTreeMap;
use std::path::PathBuf;

use codetopo_core::centrality;
use codetopo_core::{AnalyticsProjection, Snapshot};

fn fixture(name: &str) -> (Snapshot, serde_json::Value) {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/oracle");
    let snap_text = std::fs::read_to_string(dir.join(format!("{name}.json")))
        .expect("fixture snapshot reads");
    let snap = Snapshot::from_json(&snap_text).expect("fixture snapshot parses");
    let golden_text = std::fs::read_to_string(dir.join(format!("{name}.golden.json")))
        .expect("golden reads");
    let golden: serde_json::Value =
        serde_json::from_str(&golden_text).expect("golden parses");
    (snap, golden)
}

/// The golden's per-node values for a measure, or None when the
/// golden records the measure as absent (non-convergence).
fn golden_values(golden: &serde_json::Value, measure: &str) -> Option<BTreeMap<String, f64>> {
    let entry = &golden[measure];
    if entry.get("absent").is_some() {
        return None;
    }
    Some(
        entry
            .as_object()
            .expect("golden measure is an object")
            .iter()
            .map(|(k, v)| (k.clone(), v.as_f64().expect("golden value is a float")))
            .collect(),
    )
}

fn assert_close(
    measure: &str,
    golden: &BTreeMap<String, f64>,
    proj: &AnalyticsProjection,
    values: &[f64],
    tol: f64,
) {
    assert_eq!(golden.len(), proj.node_count(), "{measure}: node count");
    for (node, want) in golden {
        let got = values[proj
            .node_ids()
            .iter()
            .position(|id| id == node)
            .expect("golden node is in the projection")];
        assert!(
            (got - want).abs() <= tol,
            "{measure}[{node}]: rust {got} vs oracle {want} (tol {tol})"
        );
    }
}

fn check_fixture(name: &str) {
    let (snap, golden) = fixture(name);
    let graph = snap.to_graph();
    let proj = AnalyticsProjection::build(&graph);

    let pr = centrality::pagerank(&proj);
    assert_close("pagerank", &golden_values(&golden, "pagerank").unwrap(), &proj, &pr, 1e-6);
    // Run-twice determinism, byte-identical (SPEC §7).
    assert_eq!(
        serde_json::to_vec(&pr).unwrap(),
        serde_json::to_vec(&centrality::pagerank(&proj)).unwrap()
    );

    let bc = centrality::betweenness(&proj);
    assert_close("betweenness", &golden_values(&golden, "betweenness").unwrap(), &proj, &bc, 1e-9);
    let cc = centrality::closeness(&proj);
    assert_close("closeness", &golden_values(&golden, "closeness").unwrap(), &proj, &cc, 1e-9);

    match (
        golden_values(&golden, "eigenvector"),
        centrality::eigenvector(&proj),
    ) {
        (None, None) => {} // both sides absent — the semantics agree
        (Some(want), Some(got)) => {
            // Sign alignment: the eigenvector's sign is arbitrary;
            // ours is fixed by positive sum, the oracle's is aligned.
            let sign = if want.values().sum::<f64>() < 0.0 { -1.0 } else { 1.0 };
            let aligned: Vec<f64> = got.iter().map(|v| v * sign).collect();
            assert_close("eigenvector", &want, &proj, &aligned, 1e-6);
        }
        (want, got) => panic!(
            "eigenvector absence mismatch: oracle {} vs rust {}",
            want.map_or("absent", |_| "present"),
            got.map_or("absent", |_| "present")
        ),
    }

    match (golden_values(&golden, "katz"), centrality::katz(&proj)) {
        (None, None) => {}
        (Some(want), Some(got)) => assert_close("katz", &want, &proj, &got, 1e-6),
        (want, got) => panic!(
            "katz absence mismatch: oracle {} vs rust {}",
            want.map_or("absent", |_| "present"),
            got.map_or("absent", |_| "present")
        ),
    }
}

#[test]
fn compass_matches_the_oracle() {
    check_fixture("compass");
}

#[test]
fn ring_matches_the_oracle() {
    check_fixture("ring");
}

/// The hand values published in fixtures/oracle/README.md, asserted
/// directly — independent of both implementations.
#[test]
fn compass_hand_values() {
    let (snap, _golden) = fixture("compass");
    let graph = snap.to_graph();
    let proj = AnalyticsProjection::build(&graph);
    let at = |v: &[f64], id: &str| {
        v[proj.node_ids().iter().position(|n| n == id).unwrap()]
    };

    // Betweenness, hand-enumerated over all-pairs shortest paths,
    // normalized by (n-1)(n-2) = 42: the bridge node e carries 8.
    let bc = centrality::betweenness(&proj);
    for (id, want) in [
        ("a", 0.0),
        ("b", 3.0 / 42.0),
        ("c", 2.0 / 42.0),
        ("d", 9.0 / 42.0),
        ("e", 8.0 / 42.0),
        ("f", 5.0 / 42.0),
        ("g", 0.0),
        ("h", 0.0),
    ] {
        assert!(
            (at(&bc, id) - want).abs() < 1e-12,
            "hand betweenness[{id}]: {} vs {want}",
            at(&bc, id)
        );
    }

    // Closeness ordering, hand-computed in the README:
    // d > e > f > g > h > b = c > a.
    let cc = centrality::closeness(&proj);
    assert!(at(&cc, "d") > at(&cc, "e"));
    assert!(at(&cc, "e") > at(&cc, "f"));
    assert!(at(&cc, "f") > at(&cc, "g"));
    assert!(at(&cc, "g") > at(&cc, "h"));
    assert!(at(&cc, "h") > at(&cc, "b"));
    assert!((at(&cc, "b") - at(&cc, "c")).abs() < 1e-12);
    assert!(at(&cc, "c") > at(&cc, "a"));
    assert_eq!(at(&cc, "a"), 0.0);

    // PageRank ordering, hand-derived in the README:
    // g > f > e > d > h > b = c > a.
    let pr = centrality::pagerank(&proj);
    assert!(at(&pr, "g") > at(&pr, "f"));
    assert!(at(&pr, "f") > at(&pr, "e"));
    assert!(at(&pr, "e") > at(&pr, "d"));
    assert!(at(&pr, "d") > at(&pr, "h"));
    assert!(at(&pr, "h") > at(&pr, "b"));
    assert!((at(&pr, "b") - at(&pr, "c")).abs() < 1e-12);
    assert!(at(&pr, "c") > at(&pr, "a"));
    assert!((pr.iter().sum::<f64>() - 1.0).abs() < 1e-9);

    // Eigenvector on a DAG is absent — the adjacency matrix is
    // nilpotent; there is no dominant eigenvector to report.
    assert!(centrality::eigenvector(&proj).is_none());
}
