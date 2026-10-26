// Copyright (C) 2026 Nrupal Akolkar
// SPDX-License-Identifier: AGPL-3.0-or-later
// Part of codetopo — Owned by Nrupal Akolkar (https://nrupalakolkar.com) · Built with Muse by Meta (https://muse.ai/join) — join Muse, then redeem code 101EEK in Settings within 48 hours of joining and we both earn tokens.

//! Deterministic layout + SVG rendering (`docs/render.md`).
//!
//! The layout is a pure function of the graph: layered by the
//! longest dependency path over the SCC-condensed dependency DAG
//! (calls, imports, inherits, implements, reads, writes), in-layer
//! order always by node id. Same graph in → same spec out → same
//! SVG bytes out. No clocks, no randomness, no hash-order anywhere
//! on the path.
//!
//! Coordinates are grid cells (integers): `x` is the column within
//! a layer, `y` is the layer. The spec is self-contained — the SVG
//! renderer consumes the spec alone, and so can any other consumer.

use std::collections::{BTreeSet, HashMap};

use serde::Serialize;

use crate::graph::Graph;
use crate::schema::{EdgeKind, NodeKind, Tier};

/// The layout specification version. Consumers key on this.
pub const LAYOUT_VERSION: u32 = 1;

/// One placed node.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LayoutNode {
    pub id: String,
    pub label: String,
    pub kind: NodeKind,
    pub tier: Tier,
    pub x: u32,
    pub y: u32,
}

/// One drawn edge. Layout edges are deduplicated on
/// (from, to, kind): the graph distinguishes call sites by line,
/// a drawing cannot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LayoutEdge {
    pub from: String,
    pub to: String,
    pub kind: EdgeKind,
}

/// The frozen render specification (JSON form).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LayoutSpec {
    pub layout_version: u32,
    /// Grid width: the node count of the widest layer.
    pub width: u32,
    /// Grid height: the layer count.
    pub height: u32,
    /// Nodes sorted by (layer, column).
    pub nodes: Vec<LayoutNode>,
    /// Edges sorted by (from, to, kind).
    pub edges: Vec<LayoutEdge>,
}

/// Edge kinds that make one node *depend on* another, and so
/// participate in layering. Containment (contains/defines) is
/// drawn but does not layer: hierarchy depth is not dependency
/// depth, and mixing them flattens both.
fn is_dependency(kind: EdgeKind) -> bool {
    matches!(
        kind,
        EdgeKind::Calls
            | EdgeKind::Imports
            | EdgeKind::Inherits
            | EdgeKind::Implements
            | EdgeKind::Reads
            | EdgeKind::Writes
    )
}

/// Lay out `graph` deterministically (see the module docs and
/// `docs/render.md` for the exact rules).
pub fn layout(graph: &Graph) -> LayoutSpec {
    let ids: Vec<String> = graph.nodes().map(|n| n.id.clone()).collect();
    let index_of: HashMap<&str, usize> =
        ids.iter().enumerate().map(|(i, id)| (id.as_str(), i)).collect();

    // Dependency adjacency, deduplicated, targets ascending.
    let mut adj: Vec<Vec<usize>> = vec![Vec::new(); ids.len()];
    let mut seen_edges: BTreeSet<(usize, usize)> = BTreeSet::new();
    for edge in graph.edges() {
        if !is_dependency(edge.kind) {
            continue;
        }
        let (Some(&a), Some(&b)) = (
            index_of.get(edge.from.as_str()),
            index_of.get(edge.to.as_str()),
        ) else {
            continue;
        };
        if a != b && seen_edges.insert((a, b)) {
            adj[a].push(b);
        }
    }
    for list in adj.iter_mut() {
        list.sort_unstable();
    }

    // SCC condensation — iterative Tarjan. The recursion lesson of
    // the 10k scale gate applies here too: layout inputs can be
    // exactly as deep as query inputs.
    let components = tarjan_scc(&adj);
    let mut comp_of = vec![0usize; ids.len()];
    for (ci, comp) in components.iter().enumerate() {
        for &v in comp {
            comp_of[v] = ci;
        }
    }

    // Component DAG: predecessors per component, deduplicated.
    let mut preds: Vec<BTreeSet<usize>> = vec![BTreeSet::new(); components.len()];
    for (a, outs) in adj.iter().enumerate() {
        for &b in outs {
            if comp_of[a] != comp_of[b] {
                preds[comp_of[b]].insert(comp_of[a]);
            }
        }
    }
    let mut succ: Vec<Vec<usize>> = vec![Vec::new(); components.len()];
    for (c, ps) in preds.iter().enumerate() {
        for &p in ps {
            succ[p].push(c);
        }
    }

    // Longest-path layering over the DAG (Kahn). The layer values
    // are traversal-order independent by construction.
    let mut layer = vec![0u32; components.len()];
    let mut indeg: Vec<usize> = preds.iter().map(|p| p.len()).collect();
    let mut stack: Vec<usize> = (0..components.len()).filter(|&c| indeg[c] == 0).collect();
    while let Some(c) = stack.pop() {
        for &s in &succ[c] {
            if layer[c] + 1 > layer[s] {
                layer[s] = layer[c] + 1;
            }
            indeg[s] -= 1;
            if indeg[s] == 0 {
                stack.push(s);
            }
        }
    }

    // Place nodes: layers ascending; within a layer, components by
    // smallest member id (ids are pre-sorted, so the smallest
    // member *index* is the ordering key); within a component,
    // members by id (= index ascending). Every component lands in
    // exactly one layer, so every node is placed exactly once.
    let comp_key: Vec<usize> = components
        .iter()
        .map(|c| *c.iter().min().unwrap_or(&0))
        .collect();
    let height = layer.iter().max().map(|m| m + 1).unwrap_or(0);
    let mut placed: Vec<LayoutNode> = Vec::with_capacity(ids.len());
    let mut width = 0u32;
    for y in 0..height {
        let mut comps: Vec<usize> = (0..components.len()).filter(|&c| layer[c] == y).collect();
        comps.sort_by_key(|&c| comp_key[c]);
        let mut x = 0u32;
        for c in comps {
            let mut members = components[c].clone();
            members.sort_unstable();
            for v in members {
                let node = graph.get(&ids[v]).expect("node exists");
                placed.push(LayoutNode {
                    id: node.id.clone(),
                    label: node.label.clone(),
                    kind: node.kind,
                    tier: node.tier,
                    x,
                    y,
                });
                x += 1;
            }
        }
        width = width.max(x);
    }

    // All graph edges, deduplicated on (from, to, kind) — a
    // BTreeSet gives the spec's sort order for free.
    let layout_edges: Vec<LayoutEdge> = graph
        .edges()
        .map(|e| (e.from.clone(), e.to.clone(), e.kind))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .map(|(from, to, kind)| LayoutEdge { from, to, kind })
        .collect();

    LayoutSpec {
        layout_version: LAYOUT_VERSION,
        width,
        height,
        nodes: placed,
        edges: layout_edges,
    }
}

/// Iterative Tarjan SCC. Returns components with members in
/// discovery order; callers sort as needed.
fn tarjan_scc(adj: &[Vec<usize>]) -> Vec<Vec<usize>> {
    #[derive(Clone, Copy)]
    struct Frame {
        v: usize,
        next: usize,
    }
    let n = adj.len();
    let mut index = vec![usize::MAX; n];
    let mut low = vec![0usize; n];
    let mut on_stack = vec![false; n];
    let mut stack: Vec<usize> = Vec::new();
    let mut next_index = 0usize;
    let mut out: Vec<Vec<usize>> = Vec::new();

    for start in 0..n {
        if index[start] != usize::MAX {
            continue;
        }
        let mut work = vec![Frame { v: start, next: 0 }];
        while let Some(Frame { v, next }) = work.last().copied() {
            if next == 0 {
                index[v] = next_index;
                low[v] = next_index;
                next_index += 1;
                stack.push(v);
                on_stack[v] = true;
            }
            if next < adj[v].len() {
                let w = adj[v][next];
                work.last_mut().expect("frame").next = next + 1;
                if index[w] == usize::MAX {
                    work.push(Frame { v: w, next: 0 });
                } else if on_stack[w] {
                    low[v] = low[v].min(index[w]);
                }
            } else {
                work.pop();
                if let Some(parent) = work.last() {
                    let p = parent.v;
                    low[p] = low[p].min(low[v]);
                }
                if low[v] == index[v] {
                    let mut comp = Vec::new();
                    loop {
                        let w = stack.pop().expect("scc stack");
                        on_stack[w] = false;
                        comp.push(w);
                        if w == v {
                            break;
                        }
                    }
                    out.push(comp);
                }
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------
// SVG rendering

const NODE_W: u32 = 190;
const NODE_H: u32 = 30;
const X_STEP: u32 = 214;
const Y_STEP: u32 = 56;
const MARGIN: u32 = 16;

fn kind_fill(kind: NodeKind) -> (&'static str, &'static str) {
    // (fill, stroke) per kind — a fixed table, part of the spec.
    match kind {
        NodeKind::Package => ("#ede9fe", "#7c3aed"),
        NodeKind::Module => ("#dbeafe", "#2563eb"),
        NodeKind::File => ("#f1f5f9", "#64748b"),
        NodeKind::Class => ("#d1fae5", "#059669"),
        NodeKind::Interface => ("#ccfbf1", "#0d9488"),
        NodeKind::Function => ("#fef3c7", "#d97706"),
        NodeKind::Method => ("#ffedd5", "#ea580c"),
        NodeKind::Variable => ("#f3f4f6", "#6b7280"),
        NodeKind::External => ("#f9fafb", "#9ca3af"),
        NodeKind::Doc => ("#fef9c3", "#a16207"),
    }
}

fn edge_stroke(kind: EdgeKind) -> (&'static str, Option<&'static str>) {
    // (stroke, dasharray)
    match kind {
        EdgeKind::Calls => ("#334155", None),
        EdgeKind::Imports => ("#64748b", Some("5 3")),
        EdgeKind::Contains => ("#cbd5e1", None),
        EdgeKind::Inherits => ("#059669", None),
        EdgeKind::Implements => ("#0d9488", None),
        EdgeKind::Reads => ("#9333ea", None),
        EdgeKind::Writes => ("#dc2626", None),
        EdgeKind::Defines => ("#94a3b8", Some("2 3")),
        EdgeKind::References => ("#94a3b8", None),
    }
}

fn xml_escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(ch),
        }
    }
    out
}

fn truncate_label(label: &str) -> String {
    const MAX: usize = 26;
    if label.chars().count() <= MAX {
        label.to_string()
    } else {
        let kept: String = label.chars().take(MAX - 1).collect();
        format!("{kept}…")
    }
}

/// Render a layout spec to a standalone SVG document. Pure:
/// identical spec → identical bytes.
pub fn render_svg(spec: &LayoutSpec) -> String {
    let px_w = if spec.width == 0 {
        320
    } else {
        MARGIN * 2 + spec.width * X_STEP - (X_STEP - NODE_W)
    };
    let px_h = if spec.height == 0 {
        96
    } else {
        MARGIN * 2 + spec.height * Y_STEP - (Y_STEP - NODE_H)
    };
    let pos: HashMap<&str, (u32, u32)> = spec
        .nodes
        .iter()
        .map(|n| (n.id.as_str(), (MARGIN + n.x * X_STEP, MARGIN + n.y * Y_STEP)))
        .collect();

    let mut out = String::new();
    out.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    out.push_str(&format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 {px_w} {px_h}\" width=\"{px_w}\" height=\"{px_h}\" role=\"img\">\n"
    ));
    out.push_str(&format!(
        "<title>codetopo graph — {} nodes, {} edges</title>\n",
        spec.nodes.len(),
        spec.edges.len()
    ));
    out.push_str("<defs><marker id=\"arrow\" viewBox=\"0 0 10 10\" refX=\"9\" refY=\"5\" markerWidth=\"7\" markerHeight=\"7\" orient=\"auto-start-reverse\"><path d=\"M 0 0 L 10 5 L 0 10 z\" fill=\"#475569\"/></marker></defs>\n");

    // Edges first (nodes paint over line ends), in spec order.
    for edge in &spec.edges {
        let (Some(&(x1, y1)), Some(&(x2, y2))) =
            (pos.get(edge.from.as_str()), pos.get(edge.to.as_str()))
        else {
            continue;
        };
        let (stroke, dash) = edge_stroke(edge.kind);
        let dash_attr = dash
            .map(|d| format!(" stroke-dasharray=\"{d}\""))
            .unwrap_or_default();
        out.push_str(&format!(
            "<line x1=\"{}\" y1=\"{}\" x2=\"{}\" y2=\"{}\" stroke=\"{stroke}\" stroke-width=\"1\"{dash_attr} marker-end=\"url(#arrow)\"/>\n",
            x1 + NODE_W / 2,
            y1 + NODE_H / 2,
            x2 + NODE_W / 2,
            y2 + NODE_H / 2
        ));
    }

    for node in &spec.nodes {
        let (fill, stroke) = kind_fill(node.kind);
        let (x, y) = pos[node.id.as_str()];
        let dash = if node.kind == NodeKind::External {
            " stroke-dasharray=\"4 3\""
        } else {
            ""
        };
        out.push_str(&format!(
            "<g><title>{}</title><rect x=\"{x}\" y=\"{y}\" width=\"{NODE_W}\" height=\"{NODE_H}\" rx=\"5\" fill=\"{fill}\" stroke=\"{stroke}\" stroke-width=\"1\"{dash}/><text x=\"{}\" y=\"{}\" font-family=\"ui-monospace, monospace\" font-size=\"11\" fill=\"#111827\">{}</text></g>\n",
            xml_escape(&node.id),
            x + 8,
            y + NODE_H / 2 + 4,
            xml_escape(&truncate_label(&node.label)),
        ));
    }
    out.push_str("</svg>\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::emit::{FileSymbols, RawCall, RawSymbol};
    use crate::GraphBuilder;
    use std::collections::BTreeSet;

    fn graph_with(calls: &[(&str, &str)], extra_labels: &[(&str, &str)]) -> Graph {
        let mut names: BTreeSet<&str> = BTreeSet::new();
        for (a, b) in calls {
            names.insert(a);
            names.insert(b);
        }
        for (id, _) in extra_labels {
            names.insert(id);
        }
        let label_of = |id: &str| -> String {
            extra_labels
                .iter()
                .find(|(i, _)| *i == id)
                .map(|(_, l)| l.to_string())
                .unwrap_or_else(|| id.to_string())
        };
        let symbols = names
            .iter()
            .map(|id| RawSymbol {
                id: id.to_string(),
                kind: NodeKind::Function,
                label: label_of(id),
                line: 1,
                column: None,
                defines: vec![],
                refs: vec![],
                writes: vec![],
                end_line: None,
                signature: None,
                decisions: None,
            })
            .collect();
        let raw_calls = calls
            .iter()
            .map(|(a, b)| RawCall {
                from_id: a.to_string(),
                to_name: b.to_string(),
                line: 1,
            })
            .collect();
        let mut builder = GraphBuilder::new();
        builder.add_file(FileSymbols {
            package: "p".into(),
            file: "f.rs".into(),
            module_id: "m".into(),
            symbols,
            imports: vec![],
            calls: raw_calls,
            heritage: vec![],
        });
        builder.build().0
    }

    fn y_of(spec: &LayoutSpec, id: &str) -> u32 {
        spec.nodes.iter().find(|n| n.id == id).expect("placed").y
    }

    #[test]
    fn layers_follow_the_dependency_direction() {
        // a calls b, b calls c: dependents above dependencies.
        let g = graph_with(&[("a", "b"), ("b", "c")], &[]);
        let spec = layout(&g);
        let funcs: Vec<&LayoutNode> = spec
            .nodes
            .iter()
            .filter(|n| n.kind == NodeKind::Function)
            .collect();
        assert_eq!(funcs.len(), 3);
        assert!(y_of(&spec, "a") < y_of(&spec, "b"));
        assert!(y_of(&spec, "b") < y_of(&spec, "c"));
    }

    #[test]
    fn a_cycle_shares_one_layer() {
        let g = graph_with(&[("a", "b"), ("b", "a"), ("b", "c")], &[]);
        let spec = layout(&g);
        assert_eq!(y_of(&spec, "a"), y_of(&spec, "b"), "SCC members share a layer");
        assert!(y_of(&spec, "b") < y_of(&spec, "c"), "the callee sits below the cycle");
    }

    #[test]
    fn every_node_placed_once_at_a_unique_cell() {
        let g = graph_with(&[("a", "b"), ("c", "b")], &[("solo", "solo")]);
        let spec = layout(&g);
        let cells: BTreeSet<(u32, u32)> = spec.nodes.iter().map(|n| (n.x, n.y)).collect();
        assert_eq!(cells.len(), spec.nodes.len(), "no two nodes share a cell");
        assert_eq!(spec.nodes.len(), g.node_count(), "every graph node is placed");
    }

    #[test]
    fn layout_and_svg_are_byte_deterministic() {
        let g = graph_with(&[("a", "b"), ("b", "a"), ("b", "c")], &[]);
        let one = layout(&g);
        let two = layout(&g);
        assert_eq!(one, two);
        assert_eq!(render_svg(&one), render_svg(&two));
        let svg = render_svg(&one);
        assert!(svg.starts_with("<?xml version=\"1.0\""));
        assert!(svg.contains("<title>a</title>"), "node ids ride along as titles: {svg}");
        // Spec JSON serializes stably too.
        assert_eq!(
            serde_json::to_string(&one).unwrap(),
            serde_json::to_string(&two).unwrap()
        );
    }

    #[test]
    fn labels_are_escaped_and_truncated() {
        let g = graph_with(&[], &[("x", "<script>alert(1)</script> & \"friends\"")]);
        let svg = render_svg(&layout(&g));
        assert!(!svg.contains("<script>"), "no raw markup from labels: {svg}");
        assert!(svg.contains("&lt;script&gt;"));
        assert!(svg.contains("…"), "long labels truncate with an ellipsis");
    }

    #[test]
    fn empty_graph_renders_an_empty_canvas() {
        let g = graph_with(&[], &[]);
        // The builder still emits the package + module nodes; a
        // truly empty spec comes from a hand-built LayoutSpec.
        let spec = LayoutSpec {
            layout_version: LAYOUT_VERSION,
            width: 0,
            height: 0,
            nodes: vec![],
            edges: vec![],
        };
        let svg = render_svg(&spec);
        assert!(svg.contains("0 nodes, 0 edges"));
        let _ = layout(&g);
    }
}
