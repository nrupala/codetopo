// Copyright (C) 2026 2351485 ALBERTA INC.
// SPDX-License-Identifier: AGPL-3.0-or-later
// Part of codetopo — Owned by 2351485 ALBERTA INC. (https://nrupalakolkar.com) · Built with Muse by Meta (https://muse.ai/join).

//! Centrality measures over the analytics projection (SPEC — Rust
//! analytics engine, §4a). Every measure here is a *measurement with a
//! stated method*: parameters are fixed by the method text (owner
//! decision 2, 2026-11-02), never tunable per query.
//!
//! Determinism is a correctness property (SPEC §7): rustworkx-core is
//! always called with `parallel_threshold` above any graph size — its
//! parallel Brandes accumulates per-source floats under a lock in
//! completion order, which varies in the last bits between runs — and
//! the in-house PageRank iterates in the projection's canonical edge
//! order. Every function in this module is run-twice byte-identical.
//!
//! Absence semantics (SPEC §4a): an iterative measure that does not
//! converge returns `None` — an absence answer — never a partial
//! vector. Betweenness above [`BETWEENNESS_NODE_CEILING`] is not
//! computed by the surfaces at all; the ranking layer declines and
//! names the ceiling (owner decision 3).

use rustworkx_core::centrality as rwx;

use crate::projection::AnalyticsProjection;

/// PageRank damping factor (Brin & Page 1998), fixed by the method.
pub const PAGERANK_DAMPING: f64 = 0.85;
/// Katz attenuation α (Katz 1953), fixed by the method.
pub const KATZ_ALPHA: f64 = 0.1;
/// Katz bias β, fixed by the method.
pub const KATZ_BETA: f64 = 1.0;
/// Power-iteration tolerance (L1 change against `n × tol`), fixed.
pub const ITER_TOL: f64 = 1e-6;
/// Power-iteration cap, fixed.
pub const ITER_MAX: usize = 100;

/// The v1 node ceiling for exact betweenness (owner decision 3):
/// above it, the surface declines and names this number as the reason
/// — exact Brandes is never silently replaced by an approximation.
///
/// FINALIZED at 50,000 by RA5's scale runs (release builds,
/// 2026-11-02; `docs/scale-gate.md` records them): the
/// 40,001-node / 90,017-edge scale corpus completes in ~33 s; a
/// 2,000-node complete digraph (3,998,000 edges — the density
/// extreme) in ~35 s; the RA1 probe series (1,000 nodes 0.09 s;
/// 5,000 2.6 s; 10,000 11 s) projects to roughly a minute at the
/// ceiling for realistic graphs. A node ceiling cannot bound
/// density, so a declined answer names the graph's node count
/// alongside the ceiling, and the measured budgets live in the
/// scale gate rather than in this constant.
pub const BETWEENNESS_NODE_CEILING: usize = 50_000;

/// PageRank (Brin & Page 1998): the stationary distribution of a
/// random walk with teleportation, computed by sparse power iteration
/// from the uniform vector in f64 over the projection's directed
/// simple graph. Dangling nodes (out-degree 0) redistribute their
/// share uniformly, exactly as the oracle (networkx) does.
///
/// Implemented in-house (RA0 gate ii): petgraph's `page_rank` does
/// not compute this distribution. Defined for every node of the
/// projection; the empty graph yields an empty vector.
pub fn pagerank(p: &AnalyticsProjection) -> Vec<f64> {
    let n = p.node_count();
    if n == 0 {
        return Vec::new();
    }
    let nf = n as f64;
    let mut out_deg = vec![0usize; n];
    let mut in_edges: Vec<Vec<usize>> = vec![Vec::new(); n];
    for &(a, b) in p.directed_edges() {
        out_deg[a] += 1;
        in_edges[b].push(a);
    }
    let mut x = vec![1.0 / nf; n];
    for _ in 0..ITER_MAX {
        let dangling: f64 = (0..n)
            .filter(|&j| out_deg[j] == 0)
            .map(|j| x[j])
            .sum();
        let base = (1.0 - PAGERANK_DAMPING) / nf + PAGERANK_DAMPING * dangling / nf;
        let mut next = vec![base; n];
        for i in 0..n {
            let mut acc = 0.0;
            for &j in &in_edges[i] {
                acc += x[j] / out_deg[j] as f64;
            }
            next[i] += PAGERANK_DAMPING * acc;
        }
        let l1: f64 = next.iter().zip(&x).map(|(a, b)| (a - b).abs()).sum();
        x = next;
        if l1 < nf * ITER_TOL {
            break;
        }
    }
    x
}

/// Betweenness centrality (Freeman 1977; Brandes 2001): the fraction
/// of all-pairs shortest paths passing through each node, endpoints
/// excluded, normalized to [0, 1]. Exact — no sampling. A leaf
/// measures 0.0; 0.0 is a measurement, not an absence.
///
/// The caller enforces [`BETWEENNESS_NODE_CEILING`]; this function
/// computes whatever it is given.
pub fn betweenness(p: &AnalyticsProjection) -> Vec<f64> {
    let g = p.directed_graph();
    rwx::betweenness_centrality(&g, false, true, usize::MAX)
        .into_iter()
        .map(|v| v.unwrap_or(0.0))
        .collect()
}

/// Closeness centrality, Wasserman–Faust form (Wasserman & Faust
/// 1994): (reachable−1)/(n−1) × (reachable−1)/(Σ distances), with
/// unweighted BFS distances on the directed projection (distances
/// follow the direction networkx and rustworkx both use for directed
/// closeness: paths *to* the node). A node that nothing reaches
/// measures 0.0.
pub fn closeness(p: &AnalyticsProjection) -> Vec<f64> {
    let g = p.directed_graph();
    rwx::closeness_centrality(&g, true, usize::MAX)
        .into_iter()
        .map(|v| v.unwrap_or(0.0))
        .collect()
}

/// Eigenvector centrality (Bonacich 1972): the dominant eigenvector
/// of the adjacency matrix, L2-normalized, sign fixed by positive
/// sum, by power iteration (tolerance 1e-6, max 100 iterations).
/// `None` is the absence answer for non-convergence — on a DAG the
/// iteration collapses to the zero vector, which is non-convergence,
/// never a vector of zeros.
pub fn eigenvector(p: &AnalyticsProjection) -> Option<Vec<f64>> {
    let g = p.directed_graph();
    let out: Result<Option<Vec<f64>>, ()> =
        rwx::eigenvector_centrality(&g, |_| Ok(1.0), Some(ITER_MAX), Some(ITER_TOL));
    let mut v = out.ok()??;
    if v.iter().sum::<f64>() < 0.0 {
        for x in v.iter_mut() {
            *x = -*x;
        }
    }
    Some(v)
}

/// Katz centrality (Katz 1953): counts all walks, attenuated by
/// α = 0.1 per step, plus β = 1.0 bias, by power iteration
/// (tolerance 1e-6, max 100 iterations); the converged vector is
/// L2-normalized, as the oracle's is. `None` is the absence answer
/// for non-convergence (α ≥ 1/λmax on the given graph).
pub fn katz(p: &AnalyticsProjection) -> Option<Vec<f64>> {
    let g = p.directed_graph();
    let out: Result<Option<Vec<f64>>, ()> = rwx::katz_centrality(
        &g,
        |_| Ok(1.0),
        Some(KATZ_ALPHA),
        None,
        Some(KATZ_BETA),
        Some(ITER_MAX),
        Some(ITER_TOL),
    );
    out.ok()?
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::projection::AnalyticsProjection;
    use crate::schema::{Edge, EdgeKind, Loc, Node, NodeKind, Tier};
    use crate::snapshot::Snapshot;

    fn graph_of(ids: &[&str], edges: &[(&str, &str)]) -> crate::graph::Graph {
        let nodes = ids
            .iter()
            .map(|id| Node {
                id: id.to_string(),
                kind: NodeKind::Function,
                label: id.to_string(),
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
            })
            .collect();
        let snap = Snapshot {
            schema_version: crate::snapshot::SCHEMA_VERSION.to_string(),
            nodes,
            edges: edges
                .iter()
                .map(|(a, b)| Edge {
                    from: a.to_string(),
                    to: b.to_string(),
                    kind: EdgeKind::Calls,
                    line: None,
                    attrs: Default::default(),
                })
                .collect(),
            audit_head: String::new(),
            audit_log: Vec::new(),
        };
        snap.to_graph()
    }

    #[test]
    fn pagerank_sums_to_one_and_is_deterministic() {
        // a -> b -> c, a -> c: c is the sink everything reaches.
        let g = graph_of(&["a", "b", "c"], &[("a", "b"), ("b", "c"), ("a", "c")]);
        let p = AnalyticsProjection::build(&g);
        let v = pagerank(&p);
        assert_eq!(v.len(), 3);
        assert!((v.iter().sum::<f64>() - 1.0).abs() < 1e-9);
        assert!(v[2] > v[1] && v[1] > v[0], "sink outranks the chain: {v:?}");
        crate::testkit::assert_deterministic(|| pagerank(&p));
    }

    #[test]
    fn eigenvector_on_a_dag_is_absent_not_zero() {
        let g = graph_of(&["a", "b"], &[("a", "b")]);
        let p = AnalyticsProjection::build(&g);
        assert!(eigenvector(&p).is_none());
    }

    #[test]
    fn measures_are_run_twice_identical_on_a_cycle() {
        let g = graph_of(
            &["a", "b", "c"],
            &[("a", "b"), ("b", "c"), ("c", "a"), ("a", "c")],
        );
        let p = AnalyticsProjection::build(&g);
        crate::testkit::assert_deterministic(|| betweenness(&p));
        crate::testkit::assert_deterministic(|| closeness(&p));
        crate::testkit::assert_deterministic(|| eigenvector(&p));
        crate::testkit::assert_deterministic(|| katz(&p));
        assert!(eigenvector(&p).is_some());
        assert!(katz(&p).is_some());
    }

    #[test]
    fn empty_projection_measures_are_empty() {
        let g = graph_of(&[], &[]);
        let p = AnalyticsProjection::build(&g);
        assert!(pagerank(&p).is_empty());
        assert!(betweenness(&p).is_empty());
        assert!(closeness(&p).is_empty());
    }
}
