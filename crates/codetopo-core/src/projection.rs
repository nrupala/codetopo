// Copyright (C) 2026 2351485 ALBERTA INC.
// SPDX-License-Identifier: AGPL-3.0-or-later
// Part of codetopo — Owned by 2351485 ALBERTA INC. (https://nrupalakolkar.com) · Built with Muse by Meta (https://muse.ai/join).

//! The analytics projection (SPEC — Rust analytics engine, §3): the one
//! graph every measure in the engine runs over, built by this adapter
//! from the frozen [`Graph`].
//!
//! - **Nodes:** every non-external node, in ascending id order. External
//!   stubs (and every edge touching one) are excluded — the standing
//!   precedent: a stub accumulating centrality because forty modules
//!   call the same library function is noise the product refuses to
//!   print.
//! - **Directed edges:** the CODE_SCHEMA §9 dependency set (every edge
//!   kind except `contains` — [`crate::coupling::is_dependency`]) as a
//!   *simple* graph: parallel edges collapse to one, self-loops drop.
//! - **Community edges:** the undirected projection of the same set,
//!   weighted by multiplicity (recorded dependency edges between the
//!   pair, either direction); self-loops drop.
//!
//! Canonical construction order — nodes ascending by id, edges
//! ascending by (from, to) — makes vertex indices canonical, which is
//! what makes iterative float algorithms reproducible across runs and
//! machines (doctrine: determinism is a correctness property).
//!
//! RA0 gate notes (recorded here because this module is where the
//! gates landed):
//! - **leiden-rs licence: VERIFIED.** The 0.8.1 crate manifest declares
//!   `MIT OR Apache-2.0` and ships both licence texts. Adoption behind
//!   [`CommunityDetector`] is cleared; the implementation lands in RA2.
//! - **petgraph `page_rank` does NOT compute the method §4(a) cites.**
//!   Source-read in RA0: its iteration weights the teleport term by the
//!   source node's rank and renormalizes by the result sum afterwards,
//!   which is not the Brin & Page stationary distribution networkx
//!   computes, and its cost is quadratic in the node count per
//!   iteration. PageRank is therefore implemented in-house on this
//!   projection in RA1 (sparse power iteration, f64), oracle-checked
//!   against networkx — the §4 method text governs over the §5
//!   adoption headline, exactly as the RA0 spike gate was designed to
//!   decide.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use petgraph::graph::DiGraph;
use serde::Serialize;

use crate::coupling::is_dependency;
use crate::graph::Graph;
use crate::schema::NodeKind;

/// A frozen analytics projection of a [`Graph`] (SPEC §3). Built once
/// per query; every measure of the analytics engine reads it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AnalyticsProjection {
    /// Non-external node ids, ascending. The position in this vector is
    /// the node's vertex index everywhere in the engine.
    node_ids: Vec<String>,
    /// Directed simple edges `(from, to)` as vertex indices, ascending,
    /// deduplicated, no self-loops.
    directed_edges: Vec<(usize, usize)>,
    /// Undirected community edges `(a, b, multiplicity)` with `a < b`,
    /// ascending; multiplicity counts recorded dependency edges between
    /// the pair in either direction.
    community_edges: Vec<(usize, usize, u64)>,
}

impl AnalyticsProjection {
    /// Build the projection of `graph` (SPEC §3).
    pub fn build(graph: &Graph) -> Self {
        let mut node_ids: Vec<String> = graph
            .nodes()
            .filter(|n| n.kind != NodeKind::External && !n.id.starts_with("external::"))
            .map(|n| n.id.clone())
            .collect();
        node_ids.sort();
        let index: HashMap<&str, usize> = node_ids
            .iter()
            .enumerate()
            .map(|(i, id)| (id.as_str(), i))
            .collect();

        let mut directed: BTreeSet<(usize, usize)> = BTreeSet::new();
        let mut community: BTreeMap<(usize, usize), u64> = BTreeMap::new();
        for e in graph.edges() {
            if !is_dependency(e.kind) {
                continue;
            }
            let (Some(&a), Some(&b)) = (index.get(e.from.as_str()), index.get(e.to.as_str()))
            else {
                continue; // an endpoint is an excluded external stub
            };
            if a == b {
                continue; // self-loops drop from both projections
            }
            directed.insert((a, b));
            let key = if a < b { (a, b) } else { (b, a) };
            *community.entry(key).or_insert(0) += 1;
        }

        AnalyticsProjection {
            node_ids,
            directed_edges: directed.into_iter().collect(),
            community_edges: community
                .into_iter()
                .map(|((a, b), w)| (a, b, w))
                .collect(),
        }
    }

    /// Number of nodes in the projection.
    pub fn node_count(&self) -> usize {
        self.node_ids.len()
    }

    /// All node ids, ascending — position is the vertex index.
    pub fn node_ids(&self) -> &[String] {
        &self.node_ids
    }

    /// The id of the node at vertex `idx`.
    pub fn node_id(&self, idx: usize) -> &str {
        &self.node_ids[idx]
    }

    /// Directed simple edges as vertex-index pairs, ascending.
    pub fn directed_edges(&self) -> &[(usize, usize)] {
        &self.directed_edges
    }

    /// Undirected community edges `(a, b, multiplicity)`, `a < b`.
    pub fn community_edges(&self) -> &[(usize, usize, u64)] {
        &self.community_edges
    }

    /// The projection as a petgraph directed graph: vertex `i` is
    /// `node_ids()[i]`; edges inserted in canonical (ascending) order.
    /// One construction serves petgraph and rustworkx-core (the latter
    /// is built on petgraph).
    pub fn directed_graph(&self) -> DiGraph<(), ()> {
        let mut g = DiGraph::new();
        for _ in &self.node_ids {
            g.add_node(());
        }
        for &(a, b) in &self.directed_edges {
            g.add_edge(
                petgraph::graph::NodeIndex::new(a),
                petgraph::graph::NodeIndex::new(b),
                (),
            );
        }
        g
    }
}

/// The thin internal seam for community detection (SPEC §5): the
/// surfaces and the modularity computation depend on this trait, not on
/// any crate, so the detector can be replaced without touching them.
/// Labels returned are arbitrary; canonical community numbering
/// (by smallest member id, ascending) is the caller's job.
pub trait CommunityDetector {
    /// Community label per projection vertex index.
    fn detect(&self, projection: &AnalyticsProjection) -> Vec<usize>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::{Edge, Loc, Tier};
    use crate::snapshot::Snapshot;

    fn node(id: &str, kind: NodeKind) -> crate::schema::Node {
        crate::schema::Node {
            id: id.to_string(),
            kind,
            label: id.rsplit("::").next().unwrap_or(id).to_string(),
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
        }
    }

    fn edge(from: &str, to: &str, kind: crate::schema::EdgeKind) -> Edge {
        Edge {
            from: from.to_string(),
            to: to.to_string(),
            kind,
            line: None,
            attrs: Default::default(),
        }
    }

    /// a → b twice by different kinds (parallel), b → a, a → ext stub,
    /// b → b self-loop, c isolated, plus a `contains` edge that must not
    /// count as a dependency.
    fn fixture() -> Graph {
        use crate::schema::EdgeKind::*;
        let snap = Snapshot {
            schema_version: crate::snapshot::SCHEMA_VERSION.to_string(),
            nodes: vec![
                node("pkg::a", NodeKind::Function),
                node("pkg::b", NodeKind::Function),
                node("pkg::c", NodeKind::Function),
                node("external::serde", NodeKind::External),
            ],
            edges: vec![
                edge("pkg::a", "pkg::b", Calls),
                edge("pkg::a", "pkg::b", Reads),
                edge("pkg::b", "pkg::a", Calls),
                edge("pkg::a", "external::serde", Calls),
                edge("pkg::b", "pkg::b", Calls),
                edge("pkg::a", "pkg::c", Contains),
            ],
            audit_head: String::new(),
            audit_log: Vec::new(),
        };
        snap.to_graph()
    }

    #[test]
    fn projection_matches_the_spec_contract() {
        let p = AnalyticsProjection::build(&fixture());
        assert_eq!(p.node_ids(), ["pkg::a", "pkg::b", "pkg::c"]);
        // Parallel a→b collapsed; b→a kept; self-loop and the external
        // edge dropped; contains never a dependency.
        assert_eq!(p.directed_edges(), [(0, 1), (1, 0)]);
        // Multiplicity counts recorded edges either direction: a→b ×2
        // plus b→a ×1 = 3.
        assert_eq!(p.community_edges(), [(0, 1, 3)]);
    }

    #[test]
    fn projection_is_empty_for_an_empty_graph() {
        let snap = Snapshot {
            schema_version: crate::snapshot::SCHEMA_VERSION.to_string(),
            nodes: Vec::new(),
            edges: Vec::new(),
            audit_head: String::new(),
            audit_log: Vec::new(),
        };
        let p = AnalyticsProjection::build(&snap.to_graph());
        assert_eq!(p.node_count(), 0);
        assert!(p.directed_edges().is_empty());
        assert!(p.community_edges().is_empty());
    }

    #[test]
    fn projection_build_is_deterministic() {
        // The run-twice harness (SPEC §7): the same graph projects to
        // byte-identical bytes, and so does its snapshot round-trip.
        let g = fixture();
        crate::testkit::assert_deterministic(|| AnalyticsProjection::build(&g));
        let snap = Snapshot::from_graph(&g, &[]);
        let restored = snap.to_graph();
        assert_eq!(
            AnalyticsProjection::build(&g),
            AnalyticsProjection::build(&restored)
        );
    }

    #[test]
    fn rustworkx_centralities_are_serial_deterministic() {
        // RA0 gate (iii): with parallel_threshold above any graph size,
        // the rustworkx centralities accumulate serially and repeat
        // bit-for-bit. (The parallel path accumulates per-source floats
        // under a lock in completion order — doctrine forbids it.)
        let p = AnalyticsProjection::build(&fixture());
        let g = p.directed_graph();
        let run = || {
            let bc = rustworkx_core::centrality::betweenness_centrality(
                &g,
                false,
                true,
                usize::MAX,
            );
            let cc =
                rustworkx_core::centrality::closeness_centrality(&g, true, usize::MAX);
            (bc, cc)
        };
        let (bc1, cc1) = run();
        let (bc2, cc2) = run();
        assert_eq!(bc1, bc2);
        assert_eq!(cc1, cc2);
        // Fixture sanity: values exist and are finite.
        for v in bc1.iter().chain(cc1.iter()).flatten() {
            assert!(v.is_finite());
        }
    }
}
