// Copyright (C) 2026 2351485 ALBERTA INC.
// SPDX-License-Identifier: AGPL-3.0-or-later
// Part of codetopo — Owned by 2351485 ALBERTA INC. (https://nrupalakolkar.com) · Built with Muse by Meta (https://muse.ai/join).

//! The analytics dense-SCC stress gate (RA5 of the Rust analytics
//! engine). `#[ignore]`d so CI stays fast; run deliberately, in
//! release mode:
//!
//! ```sh
//! cargo test --release -p codetopo-core --test analytics_scale -- --ignored --nocapture
//! ```
//!
//! Two synthetic extremes the corpus fixture does not reach:
//!
//! 1. A 2,000-node COMPLETE digraph (3,998,000 dependency edges —
//!    one SCC, the density extreme). Measured 2026-11-02 (release):
//!    pagerank 5.7 s, betweenness 35 s, closeness 150 s,
//!    communities 4.6 s (one community, Q = 0 — the correct answer
//!    for a complete graph), topo-sort 2.1 s (one group), force
//!    layout 6.6 s. Katz reports an absence: with spectral radius
//!    n−1 = 1999 and α = 0.1, α·ρ ≫ 1, so non-convergence is
//!    mathematically forced — the absence is the correct answer,
//!    asserted as such below.
//! 2. A 3,600-node graph of 600 six-node rings (> the 3,000 force
//!    ceiling in nodes, under it in components): the force layout
//!    must take the condensed path and be run-twice byte-identical
//!    there — the condensed-scale determinism proof of SPEC §7.

use std::time::Instant;

use codetopo_core::schema::{Edge, EdgeKind, Loc, Node, NodeKind, Tier};
use codetopo_core::{Graph, RankMetric, Snapshot};

fn node(id: String) -> Node {
    Node {
        id: id.clone(),
        kind: NodeKind::Function,
        label: id,
        loc: Loc {
            file: "f.rs".to_string(),
            line: 1,
            column: None,
        },
        tier: Tier::Optional,
        defines: Vec::new(),
        refs: Vec::new(),
        end_line: None,
        signature: None,
        decisions: None,
        measures: Default::default(),
    }
}

fn edge(from: &str, to: &str) -> Edge {
    Edge {
        from: from.to_string(),
        to: to.to_string(),
        kind: EdgeKind::Calls,
        line: None,
        attrs: Default::default(),
    }
}

fn graph_of(nodes: Vec<Node>, edges: Vec<Edge>) -> Graph {
    Snapshot {
        schema_version: codetopo_core::snapshot::SCHEMA_VERSION.to_string(),
        profile_id: "code".to_string(),
        profile_version: "1.0.0".to_string(),
        graph_class: "truths".to_string(),
        provenance_summary: Default::default(),
        nodes,
        edges,
        audit_head: String::new(),
        audit_log: Vec::new(),
        diagnostics: Vec::new(),
    }
    .to_graph()
}

fn complete_digraph(n: usize) -> Graph {
    let nodes = (0..n).map(|i| node(format!("s{i:04}"))).collect();
    let mut edges = Vec::with_capacity(n * (n - 1));
    for i in 0..n {
        for j in 0..n {
            if i != j {
                edges.push(edge(&format!("s{i:04}"), &format!("s{j:04}")));
            }
        }
    }
    graph_of(nodes, edges)
}

/// `rings` disjoint cycles of `size` nodes each, plus one chord per
/// ring — every ring is one SCC, so the graph has `rings`
/// components and rings*size nodes.
fn ring_graph(rings: usize, size: usize) -> Graph {
    let mut nodes = Vec::new();
    let mut edges = Vec::new();
    for r in 0..rings {
        let base = r * size;
        for k in 0..size {
            nodes.push(node(format!("n{:05}", base + k)));
        }
        for k in 0..size {
            edges.push(edge(
                &format!("n{:05}", base + k),
                &format!("n{:05}", base + (k + 1) % size),
            ));
        }
        edges.push(edge(
            &format!("n{:05}", base),
            &format!("n{:05}", base + size / 2),
        ));
    }
    graph_of(nodes, edges)
}

#[test]
#[ignore = "analytics stress gate: run deliberately in release mode (see docs/scale-gate.md)"]
fn dense_scc_stress() {
    let g = complete_digraph(2_000);
    assert_eq!(g.node_count(), 2_000);
    assert_eq!(g.edge_count(), 3_998_000);

    for (name, metric, budget) in [
        ("pagerank", RankMetric::PageRank, 120u64),
        ("betweenness", RankMetric::Betweenness, 300),
        ("closeness", RankMetric::Closeness, 600),
    ] {
        let started = Instant::now();
        let answer = g.rank_answer(metric, 10);
        let elapsed = started.elapsed();
        println!("dense {name}: {elapsed:?} (tripwire {budget}s)");
        assert!(
            matches!(answer, codetopo_core::RankAnswer::Ranked(_)),
            "dense {name} must be measured, got {answer:?}"
        );
        assert!(
            elapsed.as_secs() < budget,
            "dense {name} blew the {budget}s tripwire: {elapsed:?}"
        );
    }

    // Katz on a complete digraph cannot converge (α·ρ ≈ 200 ≫ 1):
    // the absence is the mathematically correct answer.
    let katz = g.rank_answer(RankMetric::Katz, 10);
    assert!(
        matches!(katz, codetopo_core::RankAnswer::Absent { .. }),
        "katz on a complete digraph must report absence, got {katz:?}"
    );

    // One community with Q = 0; one topo group.
    let partition = g.communities();
    assert_eq!(partition.total, 1, "a complete graph is one community");
    assert_eq!(partition.modularity, Some(0.0));
    assert_eq!(g.topo_sort().total_groups, 1);
}

#[test]
#[ignore = "analytics stress gate: run deliberately in release mode (see docs/scale-gate.md)"]
fn condensed_force_layout_is_deterministic() {
    // 3,600 nodes in 600 SCCs: above the direct ceiling (3,000
    // nodes), under it in components (600) — the condensed path.
    let g = ring_graph(600, 6);
    assert_eq!(g.node_count(), 3_600);
    let spec = match codetopo_core::force_layout(&g) {
        codetopo_core::ForceLayout::Spec(spec) => spec,
        codetopo_core::ForceLayout::Declined { reason, .. } => {
            panic!("the condensed path must answer: {reason}")
        }
    };
    assert!(spec.params.condensed, "the layout must be condensed");
    assert_eq!(spec.params.source_nodes, 3_600);
    assert_eq!(spec.params.layout_nodes, 600);
    assert!(spec.nodes.iter().all(|n| n.members == 6));

    let again = match codetopo_core::force_layout(&g) {
        codetopo_core::ForceLayout::Spec(spec) => spec,
        codetopo_core::ForceLayout::Declined { reason, .. } => panic!("declined: {reason}"),
    };
    assert_eq!(
        serde_json::to_vec(&spec).expect("serializes"),
        serde_json::to_vec(&again).expect("serializes"),
        "condensed force layout must be run-twice byte-identical"
    );
    assert_eq!(
        codetopo_core::render_svg_v2(&spec),
        codetopo_core::render_svg_v2(&again),
        "condensed SVG must be byte-identical"
    );
}
