// Copyright (C) 2026 Nrupal Akolkar
// SPDX-License-Identifier: AGPL-3.0-or-later
// Part of codetopo — Owned by Nrupal Akolkar (https://nrupalakolkar.com) · Built with Muse by Meta (https://muse.ai/join) — join Muse, then redeem code 101EEK in Settings within 48 hours of joining and we both earn tokens.

//! Measurements over the frozen graph — the analytics-depth layer.
//!
//! Everything here is a *measurement*: a fact about the graph, computed by a
//! method stated in this module's docs and in the product documentation.
//! codetopo reports measurements; it does not score code quality and does
//! not judge code (the doctrine as amended in `docs/phase-2-design.md` §1).
//!
//! The computations live in this one module so the CLI, HTTP, and MCP
//! surfaces serialize the same structs and cannot disagree — parity by
//! construction, as with every other query.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::graph::Graph;
use crate::schema::{Edge, EdgeKind, Node};

// ---------------------------------------------------------------------------
// Symbol dossier (DESIGN §3: get_symbol = node + incident edges + counts)
// ---------------------------------------------------------------------------

/// One symbol in full: the node record plus every incident dependency edge,
/// grouped by edge kind within each direction, with the counts stated
/// before the lists (aggregate-first, CODE_SCHEMA §6).
///
/// `contains` edges are excluded: containment is hierarchy, not dependency
/// (CODE_SCHEMA §1.4), and every other traversal in this crate — paths,
/// neighborhoods, closures — excludes it for the same reason. The node's
/// own `loc` names the file that contains it. A self-loop edge appears in
/// both directions, because it is both.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SymbolDossier {
    pub node: Node,
    /// Edges leaving the node (`from` == the node), grouped by the edge
    /// kind's wire name; each group sorted by target id, then line.
    pub outgoing: BTreeMap<String, Vec<Edge>>,
    /// Edges entering the node (`to` == the node), grouped by the edge
    /// kind's wire name; each group sorted by source id, then line.
    pub incoming: BTreeMap<String, Vec<Edge>>,
    /// Total outgoing edges across all kinds (the sum of the group sizes).
    pub outgoing_count: usize,
    /// Total incoming edges across all kinds.
    pub incoming_count: usize,
}

impl Graph {
    /// The dossier of one node (DESIGN §3's `get_symbol`, completed to its
    /// specified shape). `None` when the id is not in the graph.
    ///
    /// The incident edge set is exactly the set of edges the depth-1
    /// [`Graph::neighborhood`] reports for this node: every non-contains
    /// edge with this node as an endpoint. The neighborhood test in this
    /// module pins that equality for every node of the fixture.
    pub fn dossier(&self, id: &str) -> Option<SymbolDossier> {
        let node = self.get(id)?.clone();
        let mut outgoing: BTreeMap<String, Vec<Edge>> = BTreeMap::new();
        let mut incoming: BTreeMap<String, Vec<Edge>> = BTreeMap::new();
        let mut outgoing_count = 0usize;
        let mut incoming_count = 0usize;
        for e in self.edges() {
            if e.kind == EdgeKind::Contains {
                continue;
            }
            if e.from == id {
                outgoing
                    .entry(e.kind.as_str().to_string())
                    .or_default()
                    .push(e.clone());
                outgoing_count += 1;
            }
            if e.to == id {
                incoming
                    .entry(e.kind.as_str().to_string())
                    .or_default()
                    .push(e.clone());
                incoming_count += 1;
            }
        }
        for group in outgoing.values_mut() {
            group.sort_by(|a, b| a.to.cmp(&b.to).then_with(|| a.line.cmp(&b.line)));
        }
        for group in incoming.values_mut() {
            group.sort_by(|a, b| a.from.cmp(&b.from).then_with(|| a.line.cmp(&b.line)));
        }
        Some(SymbolDossier {
            node,
            outgoing,
            incoming,
            outgoing_count,
            incoming_count,
        })
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diag::Diagnostic;
    use crate::emit::{FileSymbols, RawCall, RawSymbol};
    use crate::graph::GraphBuilder;
    use crate::schema::{Loc, NodeKind};
    use std::collections::BTreeSet;

    fn sym(id: &str, kind: NodeKind, line: u32) -> RawSymbol {
        RawSymbol {
            id: id.to_string(),
            kind,
            label: id.rsplit("::").next().unwrap_or(id).to_string(),
            line,
            column: None,
            defines: Vec::new(),
            refs: Vec::new(),
            writes: Vec::new(),
        }
    }

    fn file(
        package: &str,
        file: &str,
        module: &str,
        symbols: Vec<RawSymbol>,
        calls: Vec<RawCall>,
    ) -> FileSymbols {
        FileSymbols {
            package: package.to_string(),
            file: file.to_string(),
            module_id: module.to_string(),
            symbols,
            imports: Vec::new(),
            calls,
            heritage: Vec::new(),
        }
    }

    fn call(from: &str, to: &str, line: u32) -> RawCall {
        RawCall { from_id: from.to_string(), to_name: to.to_string(), line }
    }

    /// The planted fixture: a mutual call (a cycle), a duplicated definition
    /// (the same symbol id in a second file), and an unresolved call.
    fn planted_builder() -> GraphBuilder {
        let mut b = GraphBuilder::new();
        b.add_file(file(
            "t",
            "m.rs",
            "t::m",
            vec![
                sym("t::m::a", NodeKind::Function, 1),
                sym("t::m::b", NodeKind::Function, 5),
            ],
            vec![
                call("t::m::a", "b", 2),
                call("t::m::b", "a", 6),
                call("t::m::a", "missing_fn", 3),
            ],
        ));
        b.add_file(file(
            "t",
            "n.rs",
            "t::n",
            vec![sym("t::m::a", NodeKind::Function, 20)],
            vec![],
        ));
        b
    }

    #[test]
    fn planted_diagnostics_are_exactly_the_three_expected() {
        let (_graph, diags) = planted_builder().build();
        assert_eq!(
            diags,
            vec![
                Diagnostic::MultiDefinition {
                    symbol: "t::m::a".to_string(),
                    locations: vec![
                        Loc { file: "m.rs".to_string(), line: 1, column: None },
                        Loc { file: "n.rs".to_string(), line: 20, column: None },
                    ],
                },
                Diagnostic::UnresolvedCall {
                    from: "t::m::a".to_string(),
                    target: "missing_fn".to_string(),
                    line: 3,
                },
                Diagnostic::Cycle {
                    edge_kind: EdgeKind::Calls,
                    cycle: vec!["t::m::a".to_string(), "t::m::b".to_string()],
                },
            ]
        );
    }

    #[test]
    fn dossier_edges_match_depth1_neighborhood_for_every_node() {
        let (graph, _diags) = planted_builder().build();
        for node in graph.nodes() {
            let dossier = graph.dossier(&node.id).expect("node has a dossier");
            let mut from_dossier: BTreeSet<(String, String, EdgeKind)> = BTreeSet::new();
            for group in dossier.outgoing.values().chain(dossier.incoming.values()) {
                for e in group {
                    from_dossier.insert((e.from.clone(), e.to.clone(), e.kind));
                }
            }
            let sub = graph.neighborhood(&node.id, 1);
            let from_neighborhood: BTreeSet<(String, String, EdgeKind)> = sub
                .edges
                .iter()
                .filter(|e| e.from == node.id || e.to == node.id)
                .map(|e| (e.from.clone(), e.to.clone(), e.kind))
                .collect();
            assert_eq!(from_dossier, from_neighborhood, "node {}", node.id);
            // Counts are the group sums, never a second computation.
            assert_eq!(
                dossier.outgoing_count,
                dossier.outgoing.values().map(Vec::len).sum::<usize>()
            );
            assert_eq!(
                dossier.incoming_count,
                dossier.incoming.values().map(Vec::len).sum::<usize>()
            );
        }
    }

    #[test]
    fn dossier_groups_by_kind_and_direction_with_counts() {
        let (graph, _diags) = planted_builder().build();
        let d = graph.dossier("t::m::a").expect("a exists");
        assert_eq!(d.outgoing_count, 2);
        assert_eq!(d.incoming_count, 1);
        let out_calls = &d.outgoing["calls"];
        // Sorted by target id: the external stub sorts before t::m::b.
        assert_eq!(out_calls.len(), 2);
        assert_eq!(out_calls[0].to, "external::missing_fn");
        assert_eq!(out_calls[1].to, "t::m::b");
        let in_calls = &d.incoming["calls"];
        assert_eq!(in_calls.len(), 1);
        assert_eq!(in_calls[0].from, "t::m::b");
        // An unknown id has no dossier (the surfaces answer unknown_node).
        assert!(graph.dossier("t::m::nope").is_none());
    }

    #[test]
    fn dossier_excludes_containment() {
        let (graph, _diags) = planted_builder().build();
        // The file node's only incident edges are `contains` edges, so its
        // dossier is empty in both directions — containment is hierarchy,
        // and the node's own record still names it in full.
        let file_id = "t::/m.rs";
        let d = graph.dossier(file_id).expect("file node exists");
        assert_eq!(d.outgoing_count, 0);
        assert_eq!(d.incoming_count, 0);
        assert!(d.outgoing.is_empty() && d.incoming.is_empty());
    }
}
