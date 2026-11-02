// Copyright (C) 2026 2351485 ALBERTA INC.
// SPDX-License-Identifier: AGPL-3.0-or-later
// Part of codetopo — Owned by 2351485 ALBERTA INC. (https://nrupalakolkar.com) · Built with Muse by Meta (https://muse.ai/join) — join Muse, then redeem code 101EEK in Settings within 48 hours of joining and we both earn tokens.

//! Oracle conformance for communities (SPEC §6/§7): Leiden is
//! judged by partition quality and agreement, never label identity
//! (two correct implementations need not label alike, and the two
//! RNG streams differ by construction).
//!
//! - `two-villages` (planted structure): exact recovery of the
//!   planted partition is required, and Q must equal the published
//!   hand arithmetic (fixtures/oracle/README.md) and the oracle's
//!   networkx modularity of the same partition, both at 1e-9 — the
//!   formula cross-check for codetopo's own Q.
//! - `karate` (the literature's standard case): Q(ours) must be at
//!   least Q(igraph) − 0.005, and normalized mutual information
//!   against the igraph partition must be ≥ 0.9.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::PathBuf;

use codetopo_core::{CommunityPartition, Snapshot};

fn load(name: &str) -> (Snapshot, serde_json::Value) {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/oracle");
    let snap = Snapshot::from_json(
        &std::fs::read_to_string(dir.join(format!("{name}.json"))).expect("fixture reads"),
    )
    .expect("fixture parses");
    let golden: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(dir.join(format!("{name}.golden.json"))).expect("golden reads"),
    )
    .expect("golden parses");
    (snap, golden)
}

/// The partition as a set of member sets — label-free, so partitions
/// compare by structure, not by arbitrary label numbers.
fn partition_sets(part: &CommunityPartition) -> BTreeSet<BTreeSet<String>> {
    let mut groups: BTreeMap<usize, BTreeSet<String>> = BTreeMap::new();
    for a in &part.assignments {
        groups.entry(a.community).or_default().insert(a.id.clone());
    }
    groups.into_values().collect()
}

fn golden_sets(golden: &serde_json::Value) -> BTreeSet<BTreeSet<String>> {
    let part = &golden["communities"]["igraph_partition"];
    let mut groups: BTreeMap<i64, BTreeSet<String>> = BTreeMap::new();
    for (node, label) in part.as_object().expect("igraph partition is an object") {
        groups
            .entry(label.as_i64().expect("label is an integer"))
            .or_default()
            .insert(node.clone());
    }
    groups.into_values().collect()
}

/// Normalized mutual information between two label vectors over the
/// same items: 2·I(X;Y) / (H(X) + H(Y)), in [0, 1].
fn nmi(x: &[usize], y: &[usize]) -> f64 {
    let n = x.len() as f64;
    let mut cx: HashMap<usize, usize> = HashMap::new();
    let mut cy: HashMap<usize, usize> = HashMap::new();
    let mut cxy: HashMap<(usize, usize), usize> = HashMap::new();
    for i in 0..x.len() {
        *cx.entry(x[i]).or_insert(0) += 1;
        *cy.entry(y[i]).or_insert(0) += 1;
        *cxy.entry((x[i], y[i])).or_insert(0) += 1;
    }
    let entropy = |c: &HashMap<usize, usize>| -> f64 {
        c.values()
            .map(|&v| {
                let p = v as f64 / n;
                -p * p.ln()
            })
            .sum()
    };
    let hx = entropy(&cx);
    let hy = entropy(&cy);
    if hx == 0.0 && hy == 0.0 {
        return 1.0;
    }
    let mi: f64 = cxy
        .iter()
        .map(|(&(a, b), &v)| {
            let p = v as f64 / n;
            p * (p / ((cx[&a] as f64 / n) * (cy[&b] as f64 / n))).ln()
        })
        .sum();
    2.0 * mi / (hx + hy)
}

#[test]
fn two_villages_recovers_the_planted_partition() {
    let (snap, golden) = load("two-villages");
    let graph = snap.to_graph();
    let part = graph.communities();

    let planted: BTreeSet<BTreeSet<String>> = [
        vec!["a1", "a2", "a3", "a4", "a5"],
        vec!["b1", "b2", "b3", "b4", "b5"],
        vec!["z9"],
    ]
    .iter()
    .map(|members| members.iter().map(|s| s.to_string()).collect())
    .collect();
    assert_eq!(partition_sets(&part), planted, "planted partition recovered exactly");
    assert_eq!(partition_sets(&part), golden_sets(&golden), "igraph agrees on the planted structure");

    // Hand arithmetic (README): m = 21, each clique l = 10, d = 21,
    // singleton contributes 0: Q = 2 * (10/21 - (21/42)^2).
    let hand = 2.0 * (10.0 / 21.0 - 0.25);
    let q = part.modularity.expect("Q is measured on a graph with edges");
    assert!((q - hand).abs() <= 1e-9, "Q {q} vs hand {hand}");
    let oracle_q = golden["communities"]["igraph_modularity"]
        .as_f64()
        .expect("oracle Q recorded");
    assert!((q - oracle_q).abs() <= 1e-9, "Q {q} vs oracle {oracle_q}");

    // Canonical numbering: village A holds the smallest member id,
    // the isolated node is last.
    let by_id: HashMap<&str, usize> = part
        .assignments
        .iter()
        .map(|a| (a.id.as_str(), a.community))
        .collect();
    assert_eq!(by_id["a1"], 0);
    assert_eq!(by_id["b1"], 1);
    assert_eq!(by_id["z9"], 2);
    assert_eq!(part.total, 3);

    // Run-twice determinism, byte-identical (SPEC §7).
    assert_eq!(
        serde_json::to_vec(&part).unwrap(),
        serde_json::to_vec(&graph.communities()).unwrap()
    );
}

#[test]
fn karate_agrees_with_igraph_on_quality_and_structure() {
    let (snap, golden) = load("karate");
    let graph = snap.to_graph();
    let part = graph.communities();

    let q_ours = part.modularity.expect("Q measured on karate");
    let q_igraph = golden["communities"]["igraph_modularity"]
        .as_f64()
        .expect("oracle Q recorded");
    assert!(
        q_ours >= q_igraph - 0.005,
        "Q(ours) {q_ours} must be >= Q(igraph) {q_igraph} - 0.005"
    );

    // NMI against the igraph partition, aligned by node id order.
    let igraph = &golden["communities"]["igraph_partition"];
    let ours: Vec<usize> = part.assignments.iter().map(|a| a.community).collect();
    let theirs: Vec<usize> = part
        .assignments
        .iter()
        .map(|a| igraph[&a.id].as_u64().expect("igraph label") as usize)
        .collect();
    let agreement = nmi(&ours, &theirs);
    assert!(agreement >= 0.9, "NMI(ours, igraph) = {agreement} must be >= 0.9");

    assert_eq!(
        serde_json::to_vec(&part).unwrap(),
        serde_json::to_vec(&graph.communities()).unwrap(),
        "karate partition is run-twice identical"
    );
}
