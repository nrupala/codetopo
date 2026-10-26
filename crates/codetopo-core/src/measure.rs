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
use crate::schema::{Edge, EdgeKind, Node, NodeKind};

impl Node {
    /// Cyclomatic complexity of this symbol (CODE_SCHEMA §9):
    /// **v = decision points + 1** (McCabe 1976), the decision points
    /// counted per §3.2's enumeration. `None` when the symbol's
    /// decisions were not measured — absence, never a default of 1.
    pub fn complexity(&self) -> Option<u32> {
        self.decisions.map(|d| d + 1)
    }

    /// Size of the declaration in lines (CODE_SCHEMA §9):
    /// `end_line − line + 1`. `None` when no end line was measured (or
    /// the recorded span is malformed, which the extractors never
    /// emit — the guard keeps a corrupt record absent, not negative).
    pub fn size_lines(&self) -> Option<u32> {
        self.end_line
            .filter(|e| *e >= self.loc.line)
            .map(|e| e - self.loc.line + 1)
    }
}

/// The derived measurements of one symbol (CODE_SCHEMA §9), computed
/// from the measured fields of its record. Each field is independently
/// optional: a symbol can carry a span without a decision count (a
/// struct) or, in a hand-built graph, the reverse.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SymbolMeasurements {
    /// Cyclomatic complexity, where decisions were measured.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub complexity: Option<u32>,
    /// Declaration span in lines, where an end line was measured.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size_lines: Option<u32>,
}

impl SymbolMeasurements {
    /// The measurements of one node; `None` when neither was measured,
    /// so an unmeasured node's dossier carries no block at all.
    pub fn of(node: &Node) -> Option<Self> {
        let m = SymbolMeasurements {
            complexity: node.complexity(),
            size_lines: node.size_lines(),
        };
        if m.complexity.is_none() && m.size_lines.is_none() {
            None
        } else {
            Some(m)
        }
    }
}

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
    /// The node's own derived measurements (CODE_SCHEMA §9), present
    /// only when at least one was measured.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub measurements: Option<SymbolMeasurements>,
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
        let measurements = SymbolMeasurements::of(&node);
        Some(SymbolDossier {
            node,
            outgoing,
            incoming,
            outgoing_count,
            incoming_count,
            measurements,
        })
    }
}

// ---------------------------------------------------------------------------
// Importance ranking — measurements, ordered (doctrine: docs/phase-2-design.md
// §1, as amended 2026-10-21)
// ---------------------------------------------------------------------------

/// A ranking metric: a count over the graph, defined exactly, that nodes
/// are ordered by. Every metric carries its published definition; the
/// surfaces state it beside the numbers (`method`), because a number
/// without its method is a claim, not a measurement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RankMetric {
    /// Fan-in (Henry & Kafura, 1981): how much depends on this node —
    /// here, the number of incoming dependency edges (all kinds except
    /// `contains`; containment is hierarchy, not dependency).
    FanIn,
    /// Fan-out (Henry & Kafura, 1981): how much this node depends on —
    /// the number of outgoing dependency edges (same exclusion).
    FanOut,
    /// Blast-radius size: the number of transitive dependents of this
    /// node over calls / inherits / implements / reads / writes
    /// (CODE_SCHEMA §6's blast radius, counted).
    BlastRadius,
    /// Cyclomatic complexity: v = decision points + 1 (McCabe 1976;
    /// CODE_SCHEMA §9). Only symbols whose decision points were
    /// measured are ranked — an unmeasured symbol is absent, never
    /// ranked as 1.
    Complexity,
}

impl RankMetric {
    /// Parse a metric name. Both spellings are accepted on every
    /// surface — the wire name (`fan_in`, `fan_out`, `blast_radius`)
    /// and the CLI's hyphenated form (`fan-in`, `fan-out`,
    /// `blast-radius`) — so no surface can refuse a name another
    /// surface taught.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "fan_in" | "fan-in" => Some(Self::FanIn),
            "fan_out" | "fan-out" => Some(Self::FanOut),
            "blast_radius" | "blast-radius" => Some(Self::BlastRadius),
            "complexity" => Some(Self::Complexity),
            _ => None,
        }
    }

    /// The wire name the surfaces accept and report.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::FanIn => "fan_in",
            Self::FanOut => "fan_out",
            Self::BlastRadius => "blast_radius",
            Self::Complexity => "complexity",
        }
    }

    /// The published definition of the count, stated wherever the
    /// ranking is reported (the provenance rule: no number without
    /// its method).
    pub fn method(self) -> &'static str {
        match self {
            Self::FanIn => "fan-in (Henry & Kafura 1981): the number of incoming \
                            dependency edges — every edge kind except contains; \
                            external stub nodes are not ranked",
            Self::FanOut => "fan-out (Henry & Kafura 1981): the number of outgoing \
                             dependency edges — every edge kind except contains; \
                             external stub nodes are not ranked",
            Self::BlastRadius => "blast radius (CODE_SCHEMA §6): the number of \
                                  transitive dependents over calls, inherits, \
                                  implements, reads, and writes edges; \
                                  external stub nodes are not ranked",
            Self::Complexity => "cyclomatic complexity (McCabe 1976; CODE_SCHEMA \
                                 §9): v = decision points + 1, decision points \
                                 counted per CODE_SCHEMA §3.2; only symbols whose \
                                 decisions were measured are ranked",
        }
    }
}

/// One ranked node: its identity and its count under the metric.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RankedSymbol {
    pub id: String,
    pub kind: NodeKind,
    pub label: String,
    pub count: usize,
}

impl Graph {
    /// The top-`limit` nodes by `metric`, as a measurement: count
    /// descending, node id ascending as the tie-break, so every surface
    /// reports the same order. Nodes whose count is zero are excluded —
    /// a ranking of everything is a census, not a ranking. `external`
    /// stub nodes are excluded too: they mark the dependency boundary
    /// (CODE_SCHEMA §2), not symbols of the indexed system, and in any
    /// real corpus the unresolved-call stubs for constructor-like names
    /// (`Ok`, `Some`, `std`) would bury every actual hub under boundary
    /// noise. `limit == 0` returns an empty list.
    pub fn rank(&self, metric: RankMetric, limit: usize) -> Vec<RankedSymbol> {
        let mut counts: Vec<(String, usize)> = match metric {
            RankMetric::FanIn | RankMetric::FanOut => {
                let mut tally: std::collections::HashMap<&str, usize> =
                    std::collections::HashMap::new();
                for e in self.edges() {
                    if e.kind == EdgeKind::Contains {
                        continue;
                    }
                    let key = match metric {
                        RankMetric::FanIn => e.to.as_str(),
                        _ => e.from.as_str(),
                    };
                    *tally.entry(key).or_insert(0) += 1;
                }
                tally
                    .into_iter()
                    .map(|(id, c)| (id.to_string(), c))
                    .collect()
            }
            RankMetric::BlastRadius => self
                .nodes()
                .map(|n| (n.id.clone(), self.blast_radius(&n.id).len()))
                .collect(),
            RankMetric::Complexity => self
                .nodes()
                .filter_map(|n| n.complexity().map(|v| (n.id.clone(), v as usize)))
                .collect(),
        };
        counts.retain(|(_, c)| *c > 0);
        counts.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        let mut ranked: Vec<RankedSymbol> = counts
            .into_iter()
            .map(|(id, count)| {
                let node = self.get(&id).expect("ranked id is a node id");
                RankedSymbol {
                    id,
                    kind: node.kind,
                    label: node.label.clone(),
                    count,
                }
            })
            .filter(|r| r.kind != NodeKind::External)
            .collect();
        ranked.truncate(limit);
        ranked
    }

    /// How many nodes have a non-zero count under `metric` — the `total`
    /// the surfaces state before the (possibly truncated) ranking.
    pub fn rank_total(&self, metric: RankMetric) -> usize {
        // The full ranking without a limit is the count's definition;
        // graphs here are index-sized, so clarity wins over a second
        // counting path that could drift from `rank`.
        self.rank(metric, usize::MAX).len()
    }
}

// ---------------------------------------------------------------------------
// Change-impact diff — Bohner & Arnold (1996): "identifying the potential
// consequences of a change, or estimating what needs to be modified to
// accomplish a change", computed over two frozen graphs.
// ---------------------------------------------------------------------------

/// A node present in both graphs whose record changed (kind, label,
/// location, tier, defines, or refs). Both records are carried so a
/// consumer can show the change without re-reading either graph.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeChange {
    pub id: String,
    pub before: Node,
    pub after: Node,
}

/// The blast-radius movement of one added / removed / changed node:
/// its radius in the before graph, in the after graph, and the signed
/// delta (`after - before`). A node absent from one side has radius 0
/// there — the closure over an unknown id is empty by definition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImpactEntry {
    pub id: String,
    pub blast_radius_before: usize,
    pub blast_radius_after: usize,
    pub delta: i64,
}

/// The difference between two frozen graphs, as data.
///
/// Identity rules, stated because a diff is only as honest as its
/// identity: a node is its `id` (same id, different record = changed);
/// an edge is its full record — `(from, to, kind, line, attrs)` — so
/// an edge whose line moved appears as one removal plus one addition,
/// never as a silent edit. Every list is sorted (nodes by id, edges by
/// `(from, to, kind)`) so two runs over the same pair agree byte for
/// byte.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GraphDiff {
    pub added_nodes: Vec<Node>,
    pub removed_nodes: Vec<Node>,
    pub changed_nodes: Vec<NodeChange>,
    pub added_edges: Vec<Edge>,
    pub removed_edges: Vec<Edge>,
    /// Blast-radius movement for every added, removed, or changed
    /// node, sorted by id.
    pub impact: Vec<ImpactEntry>,
}

/// Sort key for the edge lists of a diff: `(from, to, kind rank, line)`.
/// `(from, to, kind)` is unique within a graph, so the key is total.
fn edge_key(e: &Edge) -> (String, String, u8, u32) {
    (
        e.from.clone(),
        e.to.clone(),
        e.kind.rank(),
        e.line.unwrap_or(0),
    )
}

/// Diff two frozen graphs (see [`GraphDiff`] for the identity rules).
///
/// The inputs are any two graphs this crate can hold — two indexes
/// loaded from stores, or two snapshots via
/// [`crate::snapshot::Snapshot::to_graph`]; the computation does not
/// care which, and a test pins the two paths to the same answer.
pub fn diff_graphs(before: &Graph, after: &Graph) -> GraphDiff {
    let mut added_nodes = Vec::new();
    let mut removed_nodes = Vec::new();
    let mut changed_nodes = Vec::new();
    for n in after.nodes() {
        match before.get(&n.id) {
            None => added_nodes.push(n.clone()),
            Some(old) if old != n => changed_nodes.push(NodeChange {
                id: n.id.clone(),
                before: old.clone(),
                after: n.clone(),
            }),
            _ => {}
        }
    }
    for n in before.nodes() {
        if after.get(&n.id).is_none() {
            removed_nodes.push(n.clone());
        }
    }

    // Edges: full-record set difference over lists sorted by the
    // identity triple. Same triple, different record (a moved line)
    // falls out as one removal plus one addition, as documented.
    let mut b_edges: Vec<Edge> = before.edges().cloned().collect();
    let mut a_edges: Vec<Edge> = after.edges().cloned().collect();
    b_edges.sort_by_key(edge_key);
    a_edges.sort_by_key(edge_key);
    let mut added_edges = Vec::new();
    let mut removed_edges = Vec::new();
    let (mut i, mut j) = (0usize, 0usize);
    while i < b_edges.len() && j < a_edges.len() {
        // The key tuple orders lexicographically, which is what makes
        // the two-pointer walk a set difference: equal keys pair up (and
        // a pair whose full records differ — same line, other attrs —
        // falls out as removal + addition), a lesser key exists only on
        // its own side.
        match edge_key(&b_edges[i]).cmp(&edge_key(&a_edges[j])) {
            std::cmp::Ordering::Equal => {
                if b_edges[i] != a_edges[j] {
                    removed_edges.push(b_edges[i].clone());
                    added_edges.push(a_edges[j].clone());
                }
                i += 1;
                j += 1;
            }
            std::cmp::Ordering::Less => {
                removed_edges.push(b_edges[i].clone());
                i += 1;
            }
            std::cmp::Ordering::Greater => {
                added_edges.push(a_edges[j].clone());
                j += 1;
            }
        }
    }
    removed_edges.extend(b_edges[i..].iter().cloned());
    added_edges.extend(a_edges[j..].iter().cloned());

    // Impact: every touched node's blast radius on both sides.
    let mut touched: std::collections::BTreeSet<&str> = std::collections::BTreeSet::new();
    for n in &added_nodes {
        touched.insert(n.id.as_str());
    }
    for n in &removed_nodes {
        touched.insert(n.id.as_str());
    }
    for c in &changed_nodes {
        touched.insert(c.id.as_str());
    }
    let impact = touched
        .into_iter()
        .map(|id| {
            let b = before.blast_radius(id).len();
            let a = after.blast_radius(id).len();
            ImpactEntry {
                id: id.to_string(),
                blast_radius_before: b,
                blast_radius_after: a,
                delta: a as i64 - b as i64,
            }
        })
        .collect();

    GraphDiff {
        added_nodes,
        removed_nodes,
        changed_nodes,
        added_edges,
        removed_edges,
        impact,
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
            end_line: None,
            signature: None,
            decisions: None,
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

    /// Diamond: d calls b and c; b and c both call e.
    fn diamond() -> Graph {
        let mut b = GraphBuilder::new();
        b.add_file(file(
            "t",
            "m.rs",
            "t::m",
            vec![
                sym("t::m::d", NodeKind::Function, 1),
                sym("t::m::b", NodeKind::Function, 10),
                sym("t::m::c", NodeKind::Function, 20),
                sym("t::m::e", NodeKind::Function, 30),
            ],
            vec![
                call("t::m::d", "b", 2),
                call("t::m::d", "c", 3),
                call("t::m::b", "e", 11),
                call("t::m::c", "e", 21),
            ],
        ));
        let (g, diags) = b.build();
        assert!(diags.is_empty(), "diamond is clean: {diags:?}");
        g
    }

    fn ranked_pairs(g: &Graph, metric: RankMetric, limit: usize) -> Vec<(String, usize)> {
        g.rank(metric, limit)
            .iter()
            .map(|r| (r.id.clone(), r.count))
            .collect()
    }

    #[test]
    fn rank_orders_by_count_then_id_on_the_diamond() {
        let g = diamond();
        // fan-in: e is called by b and c; b and c tie at 1, id order wins.
        assert_eq!(
            ranked_pairs(&g, RankMetric::FanIn, 20),
            vec![
                ("t::m::e".to_string(), 2),
                ("t::m::b".to_string(), 1),
                ("t::m::c".to_string(), 1),
            ]
        );
        // fan-out: d calls two; b and c call one each.
        assert_eq!(
            ranked_pairs(&g, RankMetric::FanOut, 20),
            vec![
                ("t::m::d".to_string(), 2),
                ("t::m::b".to_string(), 1),
                ("t::m::c".to_string(), 1),
            ]
        );
        // blast radius: changing e breaks b, c, and (through them) d.
        assert_eq!(
            ranked_pairs(&g, RankMetric::BlastRadius, 20),
            vec![
                ("t::m::e".to_string(), 3),
                ("t::m::b".to_string(), 1),
                ("t::m::c".to_string(), 1),
            ]
        );
        // Zero counts are excluded and the limit truncates after sorting.
        assert_eq!(g.rank_total(RankMetric::FanIn), 3);
        assert_eq!(
            ranked_pairs(&g, RankMetric::FanIn, 1),
            vec![("t::m::e".to_string(), 2)]
        );
        assert!(g.rank(RankMetric::FanIn, 0).is_empty());
        // Entries carry the node's identity, not just the count.
        let top = &g.rank(RankMetric::FanIn, 1)[0];
        assert_eq!(top.kind, NodeKind::Function);
        assert_eq!(top.label, "e");
    }

    #[test]
    fn rank_counts_cross_verify_against_raw_edges() {
        // Every ranked count must equal an independent recount from the
        // raw edge list / the closure itself — the ranking may not invent
        // numbers the graph does not contain.
        let (g, _diags) = planted_builder().build();
        for metric in [
            RankMetric::FanIn,
            RankMetric::FanOut,
            RankMetric::BlastRadius,
            RankMetric::Complexity,
        ] {
            let ranked: std::collections::HashMap<String, usize> = g
                .rank(metric, usize::MAX)
                .into_iter()
                .map(|r| (r.id, r.count))
                .collect();
            for node in g.nodes() {
                let raw = match metric {
                    RankMetric::FanIn => g
                        .edges()
                        .filter(|e| e.kind != EdgeKind::Contains && e.to == node.id)
                        .count(),
                    RankMetric::FanOut => g
                        .edges()
                        .filter(|e| e.kind != EdgeKind::Contains && e.from == node.id)
                        .count(),
                    RankMetric::BlastRadius => g.blast_radius(&node.id).len(),
                    // The planted fixture carries no measured symbols, so
                    // the recount is "absent" (0) for every node — the
                    // positive cross-check lives in
                    // complexity_ranking_and_dossier_measurements, whose
                    // expected order is the hand count.
                    RankMetric::Complexity => {
                        node.complexity().map(|v| v as usize).unwrap_or(0)
                    }
                };
                // External stubs are boundary markers, not ranked symbols:
                // their expected ranked count is "absent" whatever the raw
                // recount says (the planted fixture's stub has fan-in 1).
                let expected = if node.kind == NodeKind::External { 0 } else { raw };
                assert_eq!(
                    ranked.get(&node.id).copied().unwrap_or(0),
                    expected,
                    "{metric:?} for {}",
                    node.id
                );
            }
        }
    }

    /// The diff pair: before — a calls b (b at line 5), c stands alone.
    /// After — b moved to line 6, c removed, d added and called by a.
    fn diff_pair() -> (Graph, Graph) {
        let mut b1 = GraphBuilder::new();
        b1.add_file(file(
            "t",
            "m.rs",
            "t::m",
            vec![
                sym("t::m::a", NodeKind::Function, 1),
                sym("t::m::b", NodeKind::Function, 5),
                sym("t::m::c", NodeKind::Function, 9),
            ],
            vec![call("t::m::a", "b", 2)],
        ));
        let (before, _) = b1.build();
        let mut b2 = GraphBuilder::new();
        b2.add_file(file(
            "t",
            "m.rs",
            "t::m",
            vec![
                sym("t::m::a", NodeKind::Function, 1),
                sym("t::m::b", NodeKind::Function, 6),
                sym("t::m::d", NodeKind::Function, 12),
            ],
            vec![call("t::m::a", "b", 2), call("t::m::a", "d", 3)],
        ));
        let (after, _) = b2.build();
        (before, after)
    }

    fn edge_triples(edges: &[Edge]) -> Vec<(String, String, EdgeKind)> {
        edges
            .iter()
            .map(|e| (e.from.clone(), e.to.clone(), e.kind))
            .collect()
    }

    #[test]
    fn diff_reports_exactly_the_planted_edit() {
        let (before, after) = diff_pair();
        let diff = diff_graphs(&before, &after);
        assert_eq!(
            diff.added_nodes.iter().map(|n| n.id.as_str()).collect::<Vec<_>>(),
            vec!["t::m::d"]
        );
        assert_eq!(
            diff.removed_nodes.iter().map(|n| n.id.as_str()).collect::<Vec<_>>(),
            vec!["t::m::c"]
        );
        assert_eq!(diff.changed_nodes.len(), 1);
        let change = &diff.changed_nodes[0];
        assert_eq!(change.id, "t::m::b");
        assert_eq!(change.before.loc.line, 5);
        assert_eq!(change.after.loc.line, 6);
        // Added edges: d's contains edge, then the new call — sorted by
        // (from, to, kind), and "t::/m.rs" sorts before "t::m::a".
        assert_eq!(
            edge_triples(&diff.added_edges),
            vec![
                ("t::/m.rs".to_string(), "t::m::d".to_string(), EdgeKind::Contains),
                ("t::m::a".to_string(), "t::m::d".to_string(), EdgeKind::Calls),
            ]
        );
        // Removed edges: c's contains edge — a removed node takes its
        // containment with it, and nothing else was removed.
        assert_eq!(
            edge_triples(&diff.removed_edges),
            vec![("t::/m.rs".to_string(), "t::m::c".to_string(), EdgeKind::Contains)]
        );
        // Impact covers exactly the touched nodes, sorted by id.
        assert_eq!(
            diff.impact,
            vec![
                ImpactEntry {
                    id: "t::m::b".to_string(),
                    blast_radius_before: 1,
                    blast_radius_after: 1,
                    delta: 0,
                },
                ImpactEntry {
                    id: "t::m::c".to_string(),
                    blast_radius_before: 0,
                    blast_radius_after: 0,
                    delta: 0,
                },
                ImpactEntry {
                    id: "t::m::d".to_string(),
                    blast_radius_before: 0,
                    blast_radius_after: 1,
                    delta: 1,
                },
            ]
        );
    }

    #[test]
    fn diff_of_a_graph_with_itself_is_empty() {
        let (before, _after) = diff_pair();
        let diff = diff_graphs(&before, &before);
        assert!(diff.added_nodes.is_empty());
        assert!(diff.removed_nodes.is_empty());
        assert!(diff.changed_nodes.is_empty());
        assert!(diff.added_edges.is_empty());
        assert!(diff.removed_edges.is_empty());
        assert!(diff.impact.is_empty());
    }

    #[test]
    fn diff_over_snapshots_equals_diff_over_graphs() {
        let (before, after) = diff_pair();
        let before_snap = crate::Snapshot::from_graph(&before, &[]).to_graph();
        let after_snap = crate::Snapshot::from_graph(&after, &[]).to_graph();
        assert_eq!(diff_graphs(&before_snap, &after_snap), diff_graphs(&before, &after));
    }

    #[test]
    fn diff_edge_line_move_is_removal_plus_addition() {
        let build_with_call_line = |line: u32| {
            let mut b = GraphBuilder::new();
            b.add_file(file(
                "t",
                "m.rs",
                "t::m",
                vec![
                    sym("t::m::a", NodeKind::Function, 1),
                    sym("t::m::b", NodeKind::Function, 5),
                ],
                vec![call("t::m::a", "b", line)],
            ));
            b.build().0
        };
        let diff = diff_graphs(&build_with_call_line(2), &build_with_call_line(7));
        assert!(diff.added_nodes.is_empty() && diff.removed_nodes.is_empty());
        assert!(diff.changed_nodes.is_empty());
        assert_eq!(diff.removed_edges.len(), 1);
        assert_eq!(diff.removed_edges[0].line, Some(2));
        assert_eq!(diff.added_edges.len(), 1);
        assert_eq!(diff.added_edges[0].line, Some(7));
        assert!(diff.impact.is_empty(), "no node was touched");
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

    fn sym_measured(
        id: &str,
        kind: NodeKind,
        line: u32,
        end_line: u32,
        decisions: Option<u32>,
    ) -> RawSymbol {
        let mut s = sym(id, kind, line);
        s.end_line = Some(end_line);
        s.decisions = decisions;
        s
    }

    /// B2 exit proof at the core layer: complexity is decisions + 1,
    /// ranked count-descending with the id tie-break; unmeasured
    /// symbols are absent from the ranking (never ranked as 1), and
    /// the dossier carries a measurements block exactly when a
    /// measurement exists.
    #[test]
    fn complexity_ranking_and_dossier_measurements() {
        let mut b = GraphBuilder::new();
        b.add_file(file(
            "t",
            "m.rs",
            "t::m",
            vec![
                sym_measured("t::m::branchy", NodeKind::Function, 1, 21, Some(6)),
                sym_measured("t::m::plain", NodeKind::Function, 23, 25, Some(0)),
                sym_measured("t::m::mid", NodeKind::Function, 27, 30, Some(2)),
                sym("t::m::unmeasured", NodeKind::Function, 32),
                sym_measured("t::m::Widget", NodeKind::Class, 34, 40, None),
            ],
            vec![],
        ));
        let (graph, _diags) = b.build();

        let ranked = graph.rank(RankMetric::Complexity, 20);
        let got: Vec<(&str, usize)> =
            ranked.iter().map(|r| (r.id.as_str(), r.count)).collect();
        assert_eq!(
            got,
            vec![("t::m::branchy", 7), ("t::m::mid", 3), ("t::m::plain", 1)],
            "v = decisions + 1, count-descending"
        );
        assert_eq!(
            graph.rank_total(RankMetric::Complexity),
            3,
            "the unmeasured function and the decision-less class are not ranked"
        );
        assert_eq!(
            RankMetric::parse("complexity"),
            Some(RankMetric::Complexity),
            "the wire name parses"
        );
        assert!(
            RankMetric::Complexity.method().contains("McCabe"),
            "the method travels with the metric"
        );

        let d = graph.dossier("t::m::branchy").expect("dossier");
        let m = d.measurements.expect("measured node carries the block");
        assert_eq!(m.complexity, Some(7));
        assert_eq!(m.size_lines, Some(21), "end_line - line + 1");

        let d = graph.dossier("t::m::unmeasured").expect("dossier");
        assert!(
            d.measurements.is_none(),
            "unmeasured node: no block at all, not zeros"
        );

        let d = graph.dossier("t::m::Widget").expect("dossier");
        let m = d.measurements.expect("a span alone still measures");
        assert_eq!(m.complexity, None);
        assert_eq!(m.size_lines, Some(7));
    }

}
