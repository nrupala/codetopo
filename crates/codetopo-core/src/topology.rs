// Copyright (C) 2026 2351485 ALBERTA INC.
// SPDX-License-Identifier: AGPL-3.0-or-later
// Part of codetopo — Owned by 2351485 ALBERTA INC. (https://nrupalakolkar.com) · Built with Muse by Meta (https://muse.ai/join).

//! Topological ordering (SPEC — Rust analytics engine, §4d): the
//! safe change order of the graph. Kahn's algorithm over the
//! SCC-condensed §9 dependency DAG (components by the same
//! iterative Tarjan the layout uses). Groups are emitted in
//! dependency order — a group's dependencies always appear in
//! earlier groups — and a multi-node group is a cycle, flagged
//! `cyclic: true`: a cycle is reported as a group, never disguised
//! as an order. Deterministic by the engine's tie-breaks: members
//! sorted by id, ready groups chosen by smallest member id.

use std::collections::BTreeSet;

use serde::Serialize;

use crate::graph::Graph;
use crate::projection::AnalyticsProjection;
use crate::render::tarjan_scc;

/// The published method stated beside every topo-sort answer.
pub const TOPO_SORT_METHOD: &str = "topological order of the SCC-condensed CODE_SCHEMA §9 \
     dependency DAG (Kahn's algorithm; strongly connected components by Tarjan, \
     the condensation the layered layout already uses): groups are emitted in \
     dependency order — every dependency of a group appears in an earlier \
     group — members are sorted by id, and ready groups are taken by smallest \
     member id; a multi-node group is a cycle and is flagged cyclic, never \
     disguised as an order; self-loops are dropped by the section-3 \
     projection, so a self-referencing node appears as a singleton group";

/// One emitted group of the order: a single node, or a strongly
/// connected component (a cycle) with its members sorted by id.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TopoGroup {
    pub members: Vec<String>,
    /// True when the group has more than one member — it is a
    /// dependency cycle, reported as a group, not an order.
    pub cyclic: bool,
}

/// The full safe-order answer: groups in dependency order, plus the
/// counts a reader needs before trusting it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TopoOrder {
    /// Number of groups (components) in the order.
    pub total_groups: usize,
    /// Number of nodes across all groups.
    pub total_nodes: usize,
    /// How many groups are cycles (`cyclic: true`).
    pub cyclic_groups: usize,
    pub groups: Vec<TopoGroup>,
}

impl Graph {
    /// The topological order of this graph's §9 dependency
    /// projection, SCC-condensed. An empty graph yields an empty
    /// order — a normal answer, not an error.
    pub fn topo_sort(&self) -> TopoOrder {
        let proj = AnalyticsProjection::build(self);
        let n = proj.node_count();
        let mut adj: Vec<Vec<usize>> = vec![Vec::new(); n];
        for &(a, b) in proj.directed_edges() {
            adj[a].push(b);
        }
        let comps = tarjan_scc(&adj);
        let k = comps.len();
        let mut comp_of = vec![0usize; n];
        let mut members: Vec<Vec<usize>> = Vec::with_capacity(k);
        for (ci, comp) in comps.iter().enumerate() {
            let mut sorted = comp.clone();
            sorted.sort_unstable();
            for &v in &sorted {
                comp_of[v] = ci;
            }
            members.push(sorted);
        }

        // Dependencies per component (out-edges to other components
        // — what this component needs first), and the reverse map.
        let mut deps: Vec<BTreeSet<usize>> = vec![BTreeSet::new(); k];
        let mut dependents: Vec<BTreeSet<usize>> = vec![BTreeSet::new(); k];
        for &(a, b) in proj.directed_edges() {
            let (ca, cb) = (comp_of[a], comp_of[b]);
            if ca != cb {
                deps[ca].insert(cb);
                dependents[cb].insert(ca);
            }
        }

        // Kahn: emit a component once all its dependencies are
        // emitted; among ready components take the one whose
        // smallest member id is smallest (member indices are id
        // order in the projection, so the key is the first member's
        // index).
        let mut pending: Vec<usize> = deps.iter().map(|d| d.len()).collect();
        let mut ready: BTreeSet<(usize, usize)> = BTreeSet::new();
        for c in 0..k {
            if pending[c] == 0 {
                ready.insert((members[c].first().copied().unwrap_or(usize::MAX), c));
            }
        }
        let mut order: Vec<usize> = Vec::with_capacity(k);
        while let Some(&(_, c)) = ready.iter().next() {
            ready.remove(&(members[c].first().copied().unwrap_or(usize::MAX), c));
            order.push(c);
            for &d in &dependents[c] {
                pending[d] -= 1;
                if pending[d] == 0 {
                    ready
                        .insert((members[d].first().copied().unwrap_or(usize::MAX), d));
                }
            }
        }
        debug_assert_eq!(order.len(), k, "the condensation is a DAG; Kahn emits all of it");

        let groups: Vec<TopoGroup> = order
            .iter()
            .map(|&c| TopoGroup {
                members: members[c]
                    .iter()
                    .map(|&v| proj.node_id(v).to_string())
                    .collect(),
                cyclic: members[c].len() > 1,
            })
            .collect();
        let cyclic_groups = groups.iter().filter(|g| g.cyclic).count();
        TopoOrder {
            total_groups: groups.len(),
            total_nodes: n,
            cyclic_groups,
            groups,
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
        };
        snap.to_graph()
    }

    /// The order property, verified mechanically: for every
    /// dependency edge across groups, the dependency's group comes
    /// strictly earlier.
    fn assert_order_property(g: &Graph) {
        let order = g.topo_sort();
        let proj = AnalyticsProjection::build(g);
        let mut pos: std::collections::HashMap<&str, usize> = Default::default();
        for (i, group) in order.groups.iter().enumerate() {
            for m in &group.members {
                pos.insert(m.as_str(), i);
            }
        }
        for &(a, b) in proj.directed_edges() {
            let (ga, gb) = (pos[proj.node_id(a)], pos[proj.node_id(b)]);
            if ga != gb {
                assert!(gb < ga, "dependency {} (group {gb}) must precede {} (group {ga})",
                    proj.node_id(b), proj.node_id(a));
            }
        }
    }

    #[test]
    fn diamond_orders_dependencies_first() {
        // a depends on b and c; both depend on d.
        let g = graph_of(
            &["a", "b", "c", "d"],
            &[("a", "b"), ("a", "c"), ("b", "d"), ("c", "d")],
        );
        let order = g.topo_sort();
        let ids: Vec<Vec<&str>> = order
            .groups
            .iter()
            .map(|g| g.members.iter().map(|s| s.as_str()).collect())
            .collect();
        assert_eq!(ids, vec![vec!["d"], vec!["b"], vec!["c"], vec!["a"]]);
        assert_eq!(order.cyclic_groups, 0);
        assert_order_property(&g);
    }

    #[test]
    fn a_cycle_is_a_flagged_group_not_an_order() {
        // x -> y -> z -> x, with w depending on the cycle.
        let g = graph_of(
            &["w", "x", "y", "z"],
            &[("x", "y"), ("y", "z"), ("z", "x"), ("w", "x")],
        );
        let order = g.topo_sort();
        assert_eq!(order.total_groups, 2);
        assert_eq!(order.cyclic_groups, 1);
        assert_eq!(order.groups[0].members, ["x", "y", "z"]);
        assert!(order.groups[0].cyclic);
        assert_eq!(order.groups[1].members, ["w"]);
        assert!(!order.groups[1].cyclic);
        assert_order_property(&g);
    }

    #[test]
    fn empty_graph_orders_to_nothing() {
        let order = graph_of(&[], &[]).topo_sort();
        assert_eq!(order.total_groups, 0);
        assert_eq!(order.total_nodes, 0);
        assert!(order.groups.is_empty());
    }

    #[test]
    fn topo_sort_is_run_twice_identical() {
        let g = graph_of(
            &["a", "b", "c", "x", "y"],
            &[("a", "b"), ("b", "c"), ("x", "y"), ("y", "x"), ("a", "x")],
        );
        crate::testkit::assert_deterministic(|| g.topo_sort());
        assert_order_property(&g);
    }
}
