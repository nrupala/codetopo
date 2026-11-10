// Copyright (C) 2026 2351485 ALBERTA INC.
// SPDX-License-Identifier: AGPL-3.0-or-later
// Part of codetopo — Owned by 2351485 ALBERTA INC. (https://nrupalakolkar.com) · Built with Muse by Meta (https://muse.ai/join).

//! Community detection (SPEC — Rust analytics engine, §4b): Leiden
//! partitions of the analytics projection's undirected,
//! multiplicity-weighted graph, with codetopo's own modularity
//! computation. One computation in core; the CLI, HTTP, and MCP
//! surfaces render it identically.
//!
//! Determinism is a correctness property: the detector runs
//! sequentially (leiden-rs is built without its rayon feature, and
//! the parallel thresholds are set above any graph size besides)
//! with a fixed seed that is part of the published method, and
//! community ids are ours — assigned by smallest member id,
//! ascending — never the algorithm's arbitrary labels.

use std::collections::BTreeMap;

use serde::Serialize;

use crate::graph::Graph;
use crate::projection::{AnalyticsProjection, CommunityDetector};

/// The fixed Leiden seed. It is part of the method (SPEC §4b), like
/// PageRank's damping 0.85 — published here and in the method text,
/// never varied per query (owner decision 2).
pub const LEIDEN_SEED: u64 = 20261102;

/// The published method stated beside every communities answer.
pub const COMMUNITIES_METHOD: &str = "Leiden community detection (Traag, Waltman & van Eck 2019), \
     modularity quality function (Newman & Girvan 2004), resolution gamma = 1.0, \
     fixed seed 20261102, over the undirected projection of the CODE_SCHEMA §9 \
     dependency set weighted by recorded-edge multiplicity (external stubs \
     excluded, self-loops dropped); community ids are assigned by smallest \
     member id, ascending; an isolated node is a singleton community; \
     modularity Q is computed by codetopo from the same projection and is \
     absent when the graph has no edges (Q is undefined at m = 0)";

/// The shipped detector: leiden-rs 0.8.1, pinned, behind the
/// [`CommunityDetector`] trait (RA0) so it can be replaced without
/// touching the surfaces. Sequential by construction — the crate is
/// a dependency with `default-features = false` (no rayon), and the
/// parallel thresholds are pinned above any graph size besides, so
/// no parallel path can engage even if the feature set changes.
pub struct LeidenDetector;

impl CommunityDetector for LeidenDetector {
    fn detect(&self, projection: &AnalyticsProjection) -> Vec<usize> {
        let n = projection.node_count();
        if n == 0 {
            return Vec::new();
        }
        let mut builder = leiden_rs::GraphDataBuilder::new(n);
        for &(a, b, w) in projection.community_edges() {
            builder
                .add_edge(a, b, w as f64)
                .expect("projection edges are in range with finite non-negative weights");
        }
        let data = builder
            .build()
            .expect("projection builds a valid leiden graph");
        let config = leiden_rs::LeidenConfig::builder()
            .resolution(1.0)
            .seed(LEIDEN_SEED)
            .quality(leiden_rs::QualityType::Modularity)
            .parallel_local_moving_threshold(usize::MAX)
            .parallel_aggregation_threshold(usize::MAX)
            .build();
        let out = leiden_rs::Leiden::new(config)
            .run(&data)
            .expect("leiden runs on the projection");
        (0..n).map(|i| out.partition.community_of(i)).collect()
    }
}

/// Modularity Q of a partition (Newman & Girvan 2004), computed by
/// codetopo from the projection: Q = Σ_c [ l_c/m − (d_c/2m)² ],
/// where l_c is the internal edge weight of community c, d_c the
/// sum of its nodes' weighted degrees, and m the total edge weight.
/// `None` when the graph has no edges — Q is undefined at m = 0 and
/// absence is reported as absence, never 0 by default.
pub fn modularity(projection: &AnalyticsProjection, labels: &[usize]) -> Option<f64> {
    let m: f64 = projection
        .community_edges()
        .iter()
        .map(|&(_, _, w)| w as f64)
        .sum();
    if m == 0.0 {
        return None;
    }
    let mut degree = vec![0.0f64; projection.node_count()];
    let mut internal: BTreeMap<usize, f64> = BTreeMap::new();
    for &(a, b, w) in projection.community_edges() {
        let w = w as f64;
        degree[a] += w;
        degree[b] += w;
        if labels[a] == labels[b] {
            *internal.entry(labels[a]).or_insert(0.0) += w;
        }
    }
    let mut degree_sum: BTreeMap<usize, f64> = BTreeMap::new();
    for (i, &label) in labels.iter().enumerate() {
        *degree_sum.entry(label).or_insert(0.0) += degree[i];
    }
    let q: f64 = degree_sum
        .iter()
        .map(|(c, d)| {
            internal.get(c).copied().unwrap_or(0.0) / m - (d / (2.0 * m)).powi(2)
        })
        .sum();
    Some(q)
}

/// One community of a partition: its canonical id and measurements.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CommunitySummary {
    /// Canonical community id (numbering by smallest member id).
    pub id: usize,
    /// Number of member nodes.
    pub size: usize,
    /// Projection edges with both ends in this community.
    pub internal_edges: usize,
    /// Projection edges with exactly one end in this community.
    pub cut_edges: usize,
}

/// One node's community assignment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct NodeCommunity {
    pub id: String,
    pub community: usize,
}

/// A full partition answer: aggregate-first (SPEC §4b) — the total,
/// the modularity (absent at m = 0), per-community summaries, and
/// every node's assignment.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CommunityPartition {
    pub total: usize,
    pub modularity: Option<f64>,
    pub communities: Vec<CommunitySummary>,
    pub assignments: Vec<NodeCommunity>,
}

impl Graph {
    /// The Leiden partition of this graph's analytics projection,
    /// with community ids canonicalized (by smallest member id,
    /// ascending), per-community measurements, and codetopo's
    /// modularity. An empty graph yields an empty partition
    /// (total 0, Q absent) — a normal answer, not an error.
    pub fn communities(&self) -> CommunityPartition {
        let proj = AnalyticsProjection::build(self);
        let labels = LeidenDetector.detect(&proj);
        let q = modularity(&proj, &labels);

        // Canonical numbering: order the label groups by their
        // smallest member node id (node ids are ascending in the
        // projection, so the first member seen is the smallest).
        let mut first_member: BTreeMap<usize, usize> = BTreeMap::new();
        for (idx, &label) in labels.iter().enumerate() {
            first_member.entry(label).or_insert(idx);
        }
        let mut groups: Vec<(usize, usize)> = first_member.into_iter().collect();
        groups.sort_by_key(|&(_, first)| first);
        let canonical: std::collections::HashMap<usize, usize> = groups
            .iter()
            .enumerate()
            .map(|(new_id, &(old_label, _))| (old_label, new_id))
            .collect();
        let canon: Vec<usize> = labels.iter().map(|l| canonical[l]).collect();

        let total = groups.len();
        let mut summaries: Vec<CommunitySummary> = (0..total)
            .map(|id| CommunitySummary {
                id,
                size: 0,
                internal_edges: 0,
                cut_edges: 0,
            })
            .collect();
        for &c in &canon {
            summaries[c].size += 1;
        }
        for &(a, b, _) in proj.community_edges() {
            if canon[a] == canon[b] {
                summaries[canon[a]].internal_edges += 1;
            } else {
                summaries[canon[a]].cut_edges += 1;
                summaries[canon[b]].cut_edges += 1;
            }
        }
        let assignments = proj
            .node_ids()
            .iter()
            .cloned()
            .zip(canon)
            .map(|(id, community)| NodeCommunity { id, community })
            .collect();
        CommunityPartition {
            total,
            modularity: q,
            communities: summaries,
            assignments,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::{Edge, EdgeKind, Loc, Node, NodeKind, Tier};
    use crate::snapshot::Snapshot;

    fn graph_of(ids: &[&str], edges: &[(&str, &str)]) -> Graph {
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
            diagnostics: Vec::new(),
        };
        snap.to_graph()
    }

    /// Two triangles joined by one bridge edge.
    fn two_triangles() -> Graph {
        graph_of(
            &["a1", "a2", "a3", "b1", "b2", "b3"],
            &[
                ("a1", "a2"),
                ("a2", "a3"),
                ("a1", "a3"),
                ("b1", "b2"),
                ("b2", "b3"),
                ("b1", "b3"),
                ("a1", "b1"),
            ],
        )
    }

    #[test]
    fn leiden_recovers_the_two_triangles() {
        let part = two_triangles().communities();
        assert_eq!(part.total, 2);
        let by_id: std::collections::HashMap<&str, usize> = part
            .assignments
            .iter()
            .map(|a| (a.id.as_str(), a.community))
            .collect();
        // Canonical numbering: the a-triangle holds the smallest id.
        assert_eq!(by_id["a1"], 0);
        assert_eq!(by_id["a2"], 0);
        assert_eq!(by_id["a3"], 0);
        assert_eq!(by_id["b1"], 1);
        assert_eq!(by_id["b2"], 1);
        assert_eq!(by_id["b3"], 1);
        assert_eq!(part.communities[0].size, 3);
        assert_eq!(part.communities[0].internal_edges, 3);
        assert_eq!(part.communities[0].cut_edges, 1);
        assert!(part.modularity.is_some());
    }

    #[test]
    fn modularity_matches_hand_arithmetic_on_two_triangles() {
        // m = 7; each triangle: l = 3, d = 7 (degrees 3,2,2 with the
        // bridge on a1/b1) — Q = 2 * (3/7 - (7/14)^2) = 2*(3/7-1/4).
        let g = two_triangles();
        let part = g.communities();
        let want = 2.0 * (3.0 / 7.0 - 0.25);
        assert!((part.modularity.unwrap() - want).abs() < 1e-12);
    }

    #[test]
    fn empty_and_edgeless_graphs_are_normal_answers() {
        let empty = graph_of(&[], &[]).communities();
        assert_eq!(empty.total, 0);
        assert!(empty.modularity.is_none());
        assert!(empty.assignments.is_empty());

        let edgeless = graph_of(&["x", "y"], &[]).communities();
        assert_eq!(edgeless.total, 2, "isolated nodes are singletons");
        assert!(edgeless.modularity.is_none(), "Q absent at m = 0");
    }

    #[test]
    fn communities_are_run_twice_identical() {
        let g = two_triangles();
        crate::testkit::assert_deterministic(|| g.communities());
    }
}
