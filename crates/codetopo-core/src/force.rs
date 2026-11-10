// Copyright (C) 2026 2351485 ALBERTA INC.
// SPDX-License-Identifier: AGPL-3.0-or-later
// Part of codetopo — Owned by 2351485 ALBERTA INC. (https://nrupalakolkar.com) · Built with Muse by Meta (https://muse.ai/join).

//! Force-directed layout — layout spec v2 (SPEC — Rust analytics
//! engine, §4c; owner decision 2026-11-02: deterministic SVG only,
//! no plotly.js export, no JavaScript in the artifact path).
//!
//! Fruchterman–Reingold (1991) implemented in-house, made a pure
//! function of the graph the way the v1 layered layout is: NO
//! randomness anywhere — initial placement is the unit circle in
//! ascending id order, the iteration count and the linear cooling
//! schedule are constants of the method (recorded in every emitted
//! spec), and all accumulation runs single-threaded in index
//! order. Same graph in → same spec bytes out → same SVG bytes
//! out, on any platform: every operation is an IEEE-754 double
//! operation in a fixed order.
//!
//! The layout runs over the same node universe and drawing edges
//! as v1; spring forces act on v1's dependency adjacency. Above
//! [`FORCE_LAYOUT_NODE_CEILING`] nodes the layout runs over the
//! SCC-condensed graph instead (each spec node is a component,
//! represented by its smallest member id, `members` recording the
//! component size), and above the ceiling in components too the
//! surface declines and names the ceiling — the layered layout
//! remains the full-graph drawing.
//!
//! Colours and grouping are NOT baked into the spec (SPEC §4c):
//! the spec is geometry plus identity; the renderer owns style.

use std::collections::{BTreeSet, HashMap};

use serde::Serialize;

use crate::graph::Graph;
use crate::render::{edge_stroke, is_dependency, kind_fill, tarjan_scc, xml_escape, LayoutEdge};
use crate::schema::{NodeKind, Tier};

/// The force layout spec version. Consumers key on this; the v1
/// layered spec (`layout_version: 1`) is untouched.
pub const FORCE_LAYOUT_VERSION: u32 = 2;

/// Iterations of the simulation — a constant of the method,
/// recorded in every emitted spec.
pub const FORCE_ITERATIONS: usize = 80;

/// The starting temperature of the linear cooling schedule
/// (`t_i = T0 · (1 − i / iterations)`), in canvas units.
pub const FORCE_INITIAL_TEMPERATURE: f64 = 100.0;

/// The square canvas the layout is normalized into.
pub const FORCE_CANVAS: f64 = 1000.0;

/// The most nodes a force layout runs over directly, and the most
/// components the condensed fallback runs over (SPEC §4c: the
/// ceilings are set from measured runs — the all-pairs repulsion
/// makes cost quadratic in the laid-out count; RA4 measured, in
/// release: 1,000 laid-out nodes ≈ 0.4 s, 3,000 ≈ 3.2 s — a few
/// seconds is the honest price of an on-demand full layout, and
/// the next step up (5,000) would cost ~9 s. RA5's scale gate
/// records the corpus behaviour). Above the ceiling in both forms
/// the surface declines with the reason.
pub const FORCE_LAYOUT_NODE_CEILING: usize = 3_000;

/// One placed node of a v2 spec. Coordinates are canvas units
/// (floats) — unlike v1's integer grid cells.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ForceNode {
    pub id: String,
    pub label: String,
    pub kind: NodeKind,
    pub tier: Tier,
    pub x: f64,
    pub y: f64,
    /// How many graph nodes this spec node stands for: 1 normally,
    /// the component size under SCC condensation.
    pub members: usize,
}

/// The run's recorded parameters: the method's constants and the
/// shape of what was laid out, so a reader can reproduce the run.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ForceParams {
    pub algorithm: &'static str,
    pub iterations: usize,
    pub initial_temperature: f64,
    /// The Fruchterman–Reingold optimal distance k = √(area / n)
    /// for this run.
    pub optimal_distance: f64,
    /// True when the layout ran over the SCC-condensed graph.
    pub condensed: bool,
    /// Nodes in the source graph.
    pub source_nodes: usize,
    /// Nodes actually laid out (components, when condensed).
    pub layout_nodes: usize,
}

/// The layout specification v2 (JSON form): geometry plus
/// identity, self-contained like v1 — the SVG renderer consumes
/// the spec alone.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LayoutSpecV2 {
    pub layout_version: u32,
    pub layout: &'static str,
    /// Canvas width in units (square canvas).
    pub width: f64,
    pub height: f64,
    /// Nodes sorted by id (the representative id, when condensed).
    pub nodes: Vec<ForceNode>,
    /// Drawing edges, deduplicated on (from, to, kind) and sorted,
    /// as in v1; endpoints are representative ids when condensed.
    pub edges: Vec<LayoutEdge>,
    pub params: ForceParams,
}

/// The outcome of asking for a force layout: the spec, or a stated
/// decline — above the ceilings the surface declines and names the
/// ceiling rather than starting an unbounded computation.
#[derive(Debug, Clone, PartialEq)]
pub enum ForceLayout {
    Spec(LayoutSpecV2),
    Declined {
        reason: String,
        source_nodes: usize,
        components: usize,
    },
}

/// Compute the force-directed layout of `graph` (see module docs).
pub fn force_layout(graph: &Graph) -> ForceLayout {
    // Node universe: every graph node, ids ascending (v1's order).
    let mut nodes: Vec<&crate::schema::Node> = graph.nodes().collect();
    nodes.sort_by(|a, b| a.id.cmp(&b.id));
    let n = nodes.len();
    let index_of: HashMap<&str, usize> = nodes
        .iter()
        .enumerate()
        .map(|(i, nd)| (nd.id.as_str(), i))
        .collect();

    // Dependency adjacency (v1's set), deduplicated, targets
    // ascending — the springs of the simulation.
    let mut adj: Vec<Vec<usize>> = vec![Vec::new(); n];
    {
        let mut seen: BTreeSet<(usize, usize)> = BTreeSet::new();
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
            if a != b && seen.insert((a, b)) {
                adj[a].push(b);
            }
        }
        for targets in &mut adj {
            targets.sort_unstable();
        }
    }

    // Components: singletons up to the ceiling; SCC condensation
    // above it; decline above the ceiling in components too.
    let (components, condensed): (Vec<Vec<usize>>, bool) = if n <= FORCE_LAYOUT_NODE_CEILING {
        ((0..n).map(|i| vec![i]).collect(), false)
    } else {
        let comps: Vec<Vec<usize>> = tarjan_scc(&adj)
            .into_iter()
            .map(|mut c| {
                c.sort_unstable();
                c
            })
            .collect();
        if comps.len() > FORCE_LAYOUT_NODE_CEILING {
            return ForceLayout::Declined {
                reason: format!(
                    "force layout declines: the graph has {n} nodes and condenses to {} \
                     components, above the force-layout ceiling of {FORCE_LAYOUT_NODE_CEILING} \
                     laid-out nodes (Fruchterman-Reingold repulsion is quadratic in the \
                     laid-out count); the layered layout remains the full-graph drawing",
                    comps.len()
                ),
                source_nodes: n,
                components: comps.len(),
            };
        }
        (comps, true)
    };
    // Placement order: components by representative (smallest id).
    let mut components = components;
    components.sort_by_key(|c| c.first().copied().unwrap_or(usize::MAX));
    let m = components.len();
    let mut comp_of = vec![0usize; n];
    for (ci, comp) in components.iter().enumerate() {
        for &v in comp {
            comp_of[v] = ci;
        }
    }

    // Component-level springs: dependency edges across components,
    // deduplicated, undirected for the simulation.
    let mut springs: Vec<(usize, usize)> = Vec::new();
    {
        let mut seen: BTreeSet<(usize, usize)> = BTreeSet::new();
        for (a, targets) in adj.iter().enumerate() {
            for &b in targets {
                let (ca, cb) = (comp_of[a], comp_of[b]);
                if ca != cb {
                    let key = if ca < cb { (ca, cb) } else { (cb, ca) };
                    if seen.insert(key) {
                        springs.push(key);
                    }
                }
            }
        }
        springs.sort_unstable();
    }

    let positions = simulate(m, &springs);
    let k = if m == 0 {
        0.0
    } else {
        (FORCE_CANVAS * FORCE_CANVAS / m as f64).sqrt()
    };

    let spec_nodes: Vec<ForceNode> = components
        .iter()
        .enumerate()
        .map(|(ci, comp)| {
            let rep = &nodes[comp[0]];
            ForceNode {
                id: rep.id.clone(),
                label: rep.label.clone(),
                kind: rep.kind,
                tier: rep.tier,
                x: positions[ci].0,
                y: positions[ci].1,
                members: comp.len(),
            }
        })
        .collect();

    // Drawing edges: v1's rule (every edge kind, deduplicated on
    // (from, to, kind), sorted), endpoints mapped to
    // representatives when condensed.
    let spec_edges: Vec<LayoutEdge> = graph
        .edges()
        .filter_map(|e| {
            let (Some(&a), Some(&b)) = (
                index_of.get(e.from.as_str()),
                index_of.get(e.to.as_str()),
            ) else {
                return None;
            };
            Some((
                nodes[components[comp_of[a]][0]].id.clone(),
                nodes[components[comp_of[b]][0]].id.clone(),
                e.kind,
            ))
        })
        .collect::<BTreeSet<_>>()
        .into_iter()
        .map(|(from, to, kind)| LayoutEdge { from, to, kind })
        .collect();

    ForceLayout::Spec(LayoutSpecV2 {
        layout_version: FORCE_LAYOUT_VERSION,
        layout: "force",
        width: FORCE_CANVAS,
        height: FORCE_CANVAS,
        nodes: spec_nodes,
        edges: spec_edges,
        params: ForceParams {
            algorithm: "fruchterman-reingold-1991",
            iterations: FORCE_ITERATIONS,
            initial_temperature: FORCE_INITIAL_TEMPERATURE,
            optimal_distance: k,
            condensed,
            source_nodes: n,
            layout_nodes: m,
        },
    })
}

/// The simulation itself. Initial placement: the circle of radius
/// 0.4 · canvas centred on the canvas, nodes in placement order.
/// Each iteration accumulates repulsion (all pairs, index order)
/// and spring attraction (edge order), then applies displacements
/// capped by the linearly cooling temperature. Finally positions
/// are normalized into the canvas with a fixed margin, preserving
/// aspect. No randomness, no parallelism, no clock.
fn simulate(m: usize, springs: &[(usize, usize)]) -> Vec<(f64, f64)> {
    if m == 0 {
        return Vec::new();
    }
    let centre = FORCE_CANVAS / 2.0;
    let radius = FORCE_CANVAS * 0.4;
    let mut pos: Vec<(f64, f64)> = (0..m)
        .map(|i| {
            if m == 1 {
                (centre, centre)
            } else {
                let theta = 2.0 * std::f64::consts::PI * (i as f64) / (m as f64);
                (centre + radius * theta.cos(), centre + radius * theta.sin())
            }
        })
        .collect();
    if m == 1 {
        return pos;
    }
    let k = (FORCE_CANVAS * FORCE_CANVAS / m as f64).sqrt();
    let mut disp = vec![(0.0f64, 0.0f64); m];
    for iter in 0..FORCE_ITERATIONS {
        for d in disp.iter_mut() {
            *d = (0.0, 0.0);
        }
        // Repulsion: every pair repels with f = k²/d.
        for i in 0..m {
            for j in (i + 1)..m {
                let dx = pos[i].0 - pos[j].0;
                let dy = pos[i].1 - pos[j].1;
                let dist = (dx * dx + dy * dy).sqrt().max(0.01);
                let force = (k * k) / dist;
                let (ux, uy) = (dx / dist, dy / dist);
                disp[i].0 += ux * force;
                disp[i].1 += uy * force;
                disp[j].0 -= ux * force;
                disp[j].1 -= uy * force;
            }
        }
        // Attraction: springs pull with f = d²/k.
        for &(a, b) in springs {
            let dx = pos[a].0 - pos[b].0;
            let dy = pos[a].1 - pos[b].1;
            let dist = (dx * dx + dy * dy).sqrt().max(0.01);
            let force = (dist * dist) / k;
            let (ux, uy) = (dx / dist, dy / dist);
            disp[a].0 -= ux * force;
            disp[a].1 -= uy * force;
            disp[b].0 += ux * force;
            disp[b].1 += uy * force;
        }
        // Apply, capped by this step's temperature.
        let temperature =
            FORCE_INITIAL_TEMPERATURE * (1.0 - (iter as f64) / (FORCE_ITERATIONS as f64));
        for i in 0..m {
            let (dx, dy) = disp[i];
            let len = (dx * dx + dy * dy).sqrt();
            if len > 0.0 {
                let step = len.min(temperature);
                pos[i].0 += (dx / len) * step;
                pos[i].1 += (dy / len) * step;
            }
        }
    }
    // Normalize into [margin, canvas − margin] preserving aspect.
    let margin = FORCE_CANVAS * 0.04;
    let (mut min_x, mut min_y) = (f64::MAX, f64::MAX);
    let (mut max_x, mut max_y) = (f64::MIN, f64::MIN);
    for &(x, y) in &pos {
        min_x = min_x.min(x);
        min_y = min_y.min(y);
        max_x = max_x.max(x);
        max_y = max_y.max(y);
    }
    let span = (max_x - min_x).max(max_y - min_y);
    if span <= 0.0 {
        return pos.iter().map(|_| (centre, centre)).collect();
    }
    let scale = (FORCE_CANVAS - 2.0 * margin) / span;
    let mid_x = (min_x + max_x) / 2.0;
    let mid_y = (min_y + max_y) / 2.0;
    pos.iter()
        .map(|&(x, y)| {
            (
                centre + (x - mid_x) * scale,
                centre + (y - mid_y) * scale,
            )
        })
        .collect()
}

/// Render a v2 spec to a complete SVG document. Mirrors the v1
/// renderer's styling (same kind fills, edge strokes, arrow
/// marker) over float canvas coordinates; a node standing for a
/// condensed component is drawn larger, by member count. Same
/// spec in → same bytes out.
pub fn render_svg_v2(spec: &LayoutSpecV2) -> String {
    let pos: HashMap<&str, (f64, f64)> = spec
        .nodes
        .iter()
        .map(|n| (n.id.as_str(), (n.x, n.y)))
        .collect();
    let mut out = String::new();
    out.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    out.push_str(&format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 {}\" width=\"{}\" height=\"{}\" role=\"img\">\n",
        fmt_num(spec.width),
        fmt_num(spec.width),
        fmt_num(spec.height),
    ));
    out.push_str(&format!(
        "<title>codetopo graph (force layout) — {} nodes, {} edges</title>\n",
        spec.nodes.len(),
        spec.edges.len()
    ));
    out.push_str("<defs><marker id=\"arrow\" viewBox=\"0 0 10 10\" refX=\"9\" refY=\"5\" markerWidth=\"7\" markerHeight=\"7\" orient=\"auto-start-reverse\"><path d=\"M 0 0 L 10 5 L 0 10 z\" fill=\"#475569\"/></marker></defs>\n");

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
            fmt_num(x1),
            fmt_num(y1),
            fmt_num(x2),
            fmt_num(y2),
        ));
    }

    for node in &spec.nodes {
        let (fill, stroke) = kind_fill(node.kind);
        let dash = if node.kind == NodeKind::External {
            " stroke-dasharray=\"4 3\""
        } else {
            ""
        };
        let r = if node.members > 1 {
            5.0 + 2.0 * (node.members as f64).log2()
        } else {
            5.0
        };
        out.push_str(&format!(
            "<g><title>{}</title><circle cx=\"{}\" cy=\"{}\" r=\"{}\" fill=\"{fill}\" stroke=\"{stroke}\" stroke-width=\"1\"{dash}/><text x=\"{}\" y=\"{}\" font-family=\"ui-monospace, monospace\" font-size=\"11\" fill=\"#111827\">{}</text></g>\n",
            xml_escape(&node.id),
            fmt_num(node.x),
            fmt_num(node.y),
            fmt_num(r),
            fmt_num(node.x + r + 3.0),
            fmt_num(node.y + 4.0),
            xml_escape(&crate::render::truncate_label(&node.label)),
        ));
    }
    out.push_str("</svg>\n");
    out
}

/// Format a coordinate deterministically: fixed 3-decimal
/// precision (SVG renderers accept it; byte stability is the
/// point), trailing zeros kept by the format itself.
fn fmt_num(v: f64) -> String {
    format!("{v:.3}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::{Edge, EdgeKind, Loc, Node, NodeKind, Tier};
    use crate::snapshot::Snapshot;
    use std::collections::BTreeSet;

    fn graph_with(calls: &[(&str, &str)]) -> Graph {
        let mut names: BTreeSet<&str> = BTreeSet::new();
        for (a, b) in calls {
            names.insert(a);
            names.insert(b);
        }
        let snap = Snapshot {
            schema_version: crate::snapshot::SCHEMA_VERSION.to_string(),
            nodes: names
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
                .collect(),
            edges: calls
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

    fn spec_of(g: &Graph) -> LayoutSpecV2 {
        match force_layout(g) {
            ForceLayout::Spec(spec) => spec,
            ForceLayout::Declined { reason, .. } => panic!("declined: {reason}"),
        }
    }

    #[test]
    fn empty_graph_lays_out_empty() {
        let spec = spec_of(&graph_with(&[]));
        assert_eq!(spec.layout_version, 2);
        assert!(spec.nodes.is_empty());
        assert_eq!(spec.params.source_nodes, 0);
    }

    #[test]
    fn coordinates_stay_on_the_canvas() {
        let spec = spec_of(&graph_with(&[("a", "b"), ("b", "c"), ("c", "a"), ("a", "d")]));
        assert_eq!(spec.nodes.len(), 4);
        for node in &spec.nodes {
            assert!(
                (0.0..=FORCE_CANVAS).contains(&node.x) && (0.0..=FORCE_CANVAS).contains(&node.y),
                "{} off canvas: {}, {}",
                node.id,
                node.x,
                node.y
            );
            assert_eq!(node.members, 1);
        }
        assert!(!spec.params.condensed);
    }

    #[test]
    fn force_layout_is_run_twice_byte_identical() {
        let g = graph_with(&[("a", "b"), ("b", "c"), ("c", "a"), ("a", "d"), ("d", "e")]);
        let first = serde_json::to_vec(&spec_of(&g)).unwrap();
        let second = serde_json::to_vec(&spec_of(&g)).unwrap();
        assert_eq!(first, second, "spec bytes must be deterministic");
        assert_eq!(render_svg_v2(&spec_of(&g)), render_svg_v2(&spec_of(&g)));
    }

    #[test]
    fn connected_nodes_end_closer_than_unconnected_ones() {
        // Two chains of two plus an isolated node: within-chain
        // distances should beat the cross distances on average.
        let spec = spec_of(&graph_with(&[("a1", "a2"), ("b1", "b2")]));
        let at = |id: &str| {
            let n = spec.nodes.iter().find(|n| n.id == id).unwrap();
            (n.x, n.y)
        };
        let dist = |p: (f64, f64), q: (f64, f64)| {
            ((p.0 - q.0).powi(2) + (p.1 - q.1).powi(2)).sqrt()
        };
        let within = (dist(at("a1"), at("a2")) + dist(at("b1"), at("b2"))) / 2.0;
        let across = dist(at("a1"), at("b1"));
        assert!(
            within < across,
            "springs should pull chains together: within {within}, across {across}"
        );
    }
}
