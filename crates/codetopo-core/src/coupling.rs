// Copyright (C) 2026 2351485 ALBERTA INC.
// SPDX-License-Identifier: AGPL-3.0-or-later
// Part of codetopo — Owned by 2351485 ALBERTA INC. (https://nrupalakolkar.com) · Built with Muse by Meta (https://muse.ai/join) — join Muse, then redeem code 101EEK in Settings within 48 hours of joining and we both earn tokens.

//! Coupling and cohesion measurements over the frozen graph — Phase B,
//! part 3. Like everything in the analytics layer, these are
//! *measurements* with a stated method (CODE_SCHEMA §9): codetopo
//! reports CBO, RFC, LCOM pair counts, TCC, ATFD, WMC, and package
//! Ca/Ce/instability as numbers beside their definitions. It attaches
//! no threshold and no verdict — there is no "God Class" finding here,
//! because judgment is not codetopo's layer.
//!
//! Type membership is by node id: a method belongs to the type whose
//! id prefixes it (`{type}::{method}`, one segment deep — the id
//! scheme the builder emits; the graph carries no class→method
//! containment edges). A type is a `class` or `interface` node.
//!
//! Cohesion is measured over **recorded accesses**: for each method,
//! the targets of its `reads`/`writes` edges plus its recorded `refs`,
//! intersected with the type's member names (its methods' labels plus
//! the labels of everything any of its methods writes). The recorded
//! data has known limits, stated here rather than smoothed over: the
//! extractors record plain assignment targets as writes, but field
//! *reads* and compound assignments (`+=`) are not recorded, and a
//! write survives in the graph only when its target resolved to a
//! symbol — so cohesion counts what the record holds, and it is
//! absent — never zero — when a type has fewer than two methods or no
//! recorded accesses at all.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use serde::{Deserialize, Serialize};

use crate::graph::Graph;
use crate::schema::{EdgeKind, Node, NodeKind};

/// The published method of every metric this module computes, keyed by
/// the field name the surfaces report. The surfaces state these beside
/// the numbers (the provenance rule: no number without its method).
pub const METRIC_METHODS: &[(&str, &str)] = &[
    (
        "cbo",
        "coupling between objects (Chidamber & Kemerer 1994): the number of \
         distinct types this type is coupled to — a dependency edge (any kind \
         except contains) between this type or one of its methods and another \
         type or one of its methods, in either direction",
    ),
    (
        "rfc",
        "response for a class (Chidamber & Kemerer 1994): the number of this \
         type's methods plus the number of distinct methods of other types \
         they call (external stubs excluded)",
    ),
    (
        "lcom",
        "lack of cohesion, as raw pair counts (Lanza & Marinescu 2006): over \
         every pair of this type's methods, a pair shares when their recorded \
         accesses overlap — the targets of their reads/writes edges and their \
         recorded references, intersected with the type's member names (its \
         methods plus the targets of its methods' writes); reported as \
         sharing and non-sharing pair counts, never collapsed to a verdict",
    ),
    (
        "tcc",
        "tight class cohesion (Bieman & Kang 1995), over recorded accesses \
         as in lcom: sharing pairs / total method pairs",
    ),
    (
        "atfd",
        "access to foreign data (Lanza & Marinescu 2006), operationalized: \
         the number of distinct symbols outside this type — variables and \
         methods — targeted by calls, reads, or writes edges from this \
         type's methods (external stubs excluded)",
    ),
    (
        "wmc",
        "weighted methods per class (Chidamber & Kemerer 1994): the sum of \
         the cyclomatic complexities of this type's methods; absent unless \
         every method's complexity was measured",
    ),
    (
        "ca",
        "afferent coupling (Martin 1994): the number of distinct \
         node-to-node dependency pairs (dependency edges per CODE_SCHEMA \
         §9: imports, calls, inherits, implements, defines, reads, writes, \
         references) from other packages into this package",
    ),
    (
        "ce",
        "efferent coupling (Martin 1994): the number of distinct \
         node-to-node dependency pairs from this package to other packages \
         of the same repository (links to external libraries are not \
         counted here; see external_ce)",
    ),
    (
        "instability",
        "instability (Martin 1994): Ce / (Ca + Ce); absent when the package \
         has no measured coupling in either direction",
    ),
    (
        "external_ce",
        "external coupling: the number of distinct node-to-node dependency \
         pairs from this package to external (third-party / stdlib) stub \
         nodes, counted and named separately from ce (CODE_SCHEMA §9, the \
         B0.3 rule) — never merged into it",
    ),
];

/// Measurements of one type (a `class` or `interface` node).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TypeMetrics {
    pub id: String,
    pub kind: NodeKind,
    pub label: String,
    /// How many methods belong to the type (id-prefix membership).
    pub method_count: usize,
    /// Distinct types coupled to this one (see [`METRIC_METHODS`]).
    pub cbo: usize,
    /// Methods of the type + distinct foreign methods they call.
    pub rfc: usize,
    /// Distinct foreign variables/methods accessed by the type's methods.
    pub atfd: usize,
    /// Sum of method complexities; `None` unless every method measured.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wmc: Option<u32>,
    /// Method pairs sharing a recorded access; `None` when cohesion
    /// was not measured (fewer than two methods, or no recorded
    /// accesses).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lcom_sharing_pairs: Option<usize>,
    /// Method pairs sharing no recorded access; `None` as above.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lcom_non_sharing_pairs: Option<usize>,
    /// Sharing pairs / total pairs; `None` as above.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tcc: Option<f64>,
}

/// Coupling measurements of one package of the indexed repository.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PackageMetrics {
    pub id: String,
    /// Distinct dependency pairs into this package from other packages.
    pub ca: usize,
    /// Distinct dependency pairs out of this package to other packages.
    pub ce: usize,
    /// Distinct dependency pairs out of this package to external stubs.
    pub external_ce: usize,
    /// Ce / (Ca + Ce); `None` when Ca + Ce == 0.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instability: Option<f64>,
}

/// All coupling/cohesion measurements of a graph, types and packages
/// each sorted by id so every surface reports the same order.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GraphMetrics {
    pub types: Vec<TypeMetrics>,
    pub packages: Vec<PackageMetrics>,
}

/// The dependency edge kinds of CODE_SCHEMA §9 — everything except
/// `contains` (hierarchy, not dependency, §1.4) — over which coupling
/// is measured.
fn is_dependency(kind: EdgeKind) -> bool {
    matches!(
        kind,
        EdgeKind::Imports
            | EdgeKind::Calls
            | EdgeKind::Inherits
            | EdgeKind::Implements
            | EdgeKind::Defines
            | EdgeKind::Reads
            | EdgeKind::Writes
            | EdgeKind::References
    )
}

/// The package a node id belongs to: the first `::` segment. `None`
/// for external stubs (they are nobody's package — links to them are
/// external coupling, not inter-package coupling).
fn package_of(id: &str) -> Option<&str> {
    if id.starts_with("external::") {
        None
    } else {
        Some(id.split("::").next().unwrap_or(id))
    }
}

impl Graph {
    /// All coupling and cohesion measurements of this graph, per
    /// [`METRIC_METHODS`] and CODE_SCHEMA §9.
    pub fn metrics(&self) -> GraphMetrics {
        // Index nodes once: by id, and methods grouped by owning type.
        let by_id: HashMap<&str, &Node> =
            self.nodes().map(|n| (n.id.as_str(), n)).collect();
        let owner_of = |node: &Node| -> Option<String> {
            match node.kind {
                NodeKind::Class | NodeKind::Interface => Some(node.id.clone()),
                NodeKind::Method => {
                    let (owner, _) = node.id.rsplit_once("::")?;
                    match by_id.get(owner) {
                        Some(t) if matches!(t.kind, NodeKind::Class | NodeKind::Interface) => {
                            Some(owner.to_string())
                        }
                        _ => None,
                    }
                }
                _ => None,
            }
        };
        // A method's owner is its immediate id parent (rsplit), so a
        // method of a nested type groups under the nested type, never
        // the outer one.
        let mut methods_of: BTreeMap<String, Vec<&Node>> = BTreeMap::new();
        for n in self.nodes() {
            if n.kind == NodeKind::Method {
                if let Some(owner) = owner_of(n) {
                    methods_of.entry(owner).or_default().push(n);
                }
            }
        }

        // One pass over the dependency edges feeds CBO, RFC, ATFD, and
        // the package pair sets.
        let mut coupled: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        let mut rfc_calls: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        let mut foreign: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        // Recorded accesses per method (for cohesion): labels of the
        // targets of its reads/writes edges; writes tracked separately
        // because written targets are the type's known data members.
        let mut access_labels: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        let mut writes_labels: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        let mut pairs_in: BTreeMap<String, BTreeSet<(String, String)>> = BTreeMap::new();
        let mut pairs_out: BTreeMap<String, BTreeSet<(String, String)>> = BTreeMap::new();
        let mut pairs_external: BTreeMap<String, BTreeSet<(String, String)>> =
            BTreeMap::new();
        for e in self.edges() {
            if !is_dependency(e.kind) {
                continue;
            }
            let (Some(from), Some(to)) = (by_id.get(e.from.as_str()), by_id.get(e.to.as_str()))
            else {
                continue;
            };
            // Package pairs (node-to-node, deduplicated by the set).
            match (package_of(&e.from), package_of(&e.to)) {
                (Some(a), Some(b)) if a != b => {
                    pairs_out
                        .entry(a.to_string())
                        .or_default()
                        .insert((e.from.clone(), e.to.clone()));
                    pairs_in
                        .entry(b.to_string())
                        .or_default()
                        .insert((e.from.clone(), e.to.clone()));
                }
                (Some(a), None) => {
                    pairs_external
                        .entry(a.to_string())
                        .or_default()
                        .insert((e.from.clone(), e.to.clone()));
                }
                _ => {}
            }
            // Type coupling: both endpoints owned by types, different.
            if let (Some(a), Some(b)) = (owner_of(from), owner_of(to)) {
                if a != b {
                    coupled.entry(a.clone()).or_default().insert(b.clone());
                    coupled.entry(b).or_default().insert(a);
                }
            }
            // Recorded accesses of a method (any owner): reads/writes
            // edge targets, by label.
            if from.kind == NodeKind::Method
                && matches!(e.kind, EdgeKind::Reads | EdgeKind::Writes)
            {
                access_labels
                    .entry(e.from.clone())
                    .or_default()
                    .insert(to.label.clone());
                if e.kind == EdgeKind::Writes {
                    writes_labels
                        .entry(e.from.clone())
                        .or_default()
                        .insert(to.label.clone());
                }
            }
            // Per-type call/access tallies, from the type's methods only.
            if from.kind == NodeKind::Method {
                if let Some(owner) = owner_of(from) {
                    if e.kind == EdgeKind::Calls
                        && to.kind == NodeKind::Method
                        && owner_of(to).as_deref() != Some(owner.as_str())
                    {
                        rfc_calls.entry(owner.clone()).or_default().insert(e.to.clone());
                    }
                    if matches!(e.kind, EdgeKind::Calls | EdgeKind::Reads | EdgeKind::Writes)
                        && matches!(to.kind, NodeKind::Method | NodeKind::Variable)
                        && owner_of(to).as_deref() != Some(owner.as_str())
                        && e.to != owner
                    {
                        foreign.entry(owner).or_default().insert(e.to.clone());
                    }
                }
            }
        }

        let mut types = Vec::new();
        for n in self.nodes() {
            if !matches!(n.kind, NodeKind::Class | NodeKind::Interface) {
                continue;
            }
            let methods = methods_of.get(&n.id).cloned().unwrap_or_default();
            let method_count = methods.len();

            // WMC: absent unless every method was measured.
            let wmc = if method_count > 0
                && methods.iter().all(|m| m.complexity().is_some())
            {
                Some(methods.iter().map(|m| m.complexity().unwrap_or(0)).sum())
            } else {
                None
            };

            // Cohesion over recorded accesses.
            let (lcom_sharing_pairs, lcom_non_sharing_pairs, tcc) =
                cohesion(&methods, &access_labels, &writes_labels);

            types.push(TypeMetrics {
                id: n.id.clone(),
                kind: n.kind,
                label: n.label.clone(),
                method_count,
                cbo: coupled.get(&n.id).map(|s| s.len()).unwrap_or(0),
                rfc: method_count
                    + rfc_calls.get(&n.id).map(|s| s.len()).unwrap_or(0),
                atfd: foreign.get(&n.id).map(|s| s.len()).unwrap_or(0),
                wmc,
                lcom_sharing_pairs,
                lcom_non_sharing_pairs,
                tcc,
            });
        }
        types.sort_by(|a, b| a.id.cmp(&b.id));

        // Packages: every package node, plus any package named by a
        // node's id prefix (a graph restored from parts may carry
        // nodes without an explicit package node).
        let mut package_ids: BTreeSet<String> = BTreeSet::new();
        for n in self.nodes() {
            if n.kind == NodeKind::Package {
                package_ids.insert(n.id.clone());
            }
            if let Some(p) = package_of(&n.id) {
                package_ids.insert(p.to_string());
            }
        }
        let mut packages = Vec::new();
        for id in package_ids {
            let ca = pairs_in.get(&id).map(|s| s.len()).unwrap_or(0);
            let ce = pairs_out.get(&id).map(|s| s.len()).unwrap_or(0);
            let external_ce = pairs_external.get(&id).map(|s| s.len()).unwrap_or(0);
            let instability = if ca + ce > 0 {
                Some(ce as f64 / (ca + ce) as f64)
            } else {
                None
            };
            packages.push(PackageMetrics {
                id,
                ca,
                ce,
                external_ce,
                instability,
            });
        }

        GraphMetrics { types, packages }
    }
}

/// LCOM pair counts and TCC for one type, over recorded accesses:
/// member names are the methods' labels plus the labels of everything
/// any method writes; a method's accessed set is its recorded refs and
/// its reads/writes edge targets, intersected with the member names.
/// Returns all-`None` when the type has fewer than two methods or no
/// recorded accesses at all — not measured, never zero.
fn cohesion(
    methods: &[&Node],
    access_labels: &BTreeMap<String, BTreeSet<String>>,
    writes_labels: &BTreeMap<String, BTreeSet<String>>,
) -> (Option<usize>, Option<usize>, Option<f64>) {
    if methods.len() < 2 {
        return (None, None, None);
    }
    let mut member_names: BTreeSet<&str> =
        methods.iter().map(|m| m.label.as_str()).collect();
    for m in methods {
        if let Some(w) = writes_labels.get(&m.id) {
            member_names.extend(w.iter().map(|s| s.as_str()));
        }
    }
    let empty = BTreeSet::new();
    let accessed: Vec<BTreeSet<&str>> = methods
        .iter()
        .map(|m| {
            let mut set: BTreeSet<&str> = m
                .refs
                .iter()
                .map(|s| s.as_str())
                .filter(|s| member_names.contains(s))
                .collect();
            for label in access_labels.get(&m.id).unwrap_or(&empty) {
                if member_names.contains(label.as_str()) {
                    set.insert(label.as_str());
                }
            }
            set
        })
        .collect();
    if accessed.iter().all(|a| a.is_empty()) {
        return (None, None, None);
    }
    let mut sharing = 0usize;
    let mut total = 0usize;
    for i in 0..accessed.len() {
        for j in (i + 1)..accessed.len() {
            total += 1;
            if !accessed[i].is_disjoint(&accessed[j]) {
                sharing += 1;
            }
        }
    }
    (
        Some(sharing),
        Some(total - sharing),
        Some(sharing as f64 / total as f64),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::{Edge, Loc, Tier};
    use crate::snapshot::Snapshot;

    fn node(id: &str, kind: NodeKind, decisions: Option<u32>, refs: &[&str]) -> Node {
        Node {
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
            refs: refs.iter().map(|s| s.to_string()).collect(),
            end_line: Some(5),
            signature: None,
            decisions,
        }
    }

    fn edge(from: &str, to: &str, kind: EdgeKind) -> Edge {
        Edge {
            from: from.to_string(),
            to: to.to_string(),
            kind,
            line: None,
            attrs: Default::default(),
        }
    }

    /// The two-package fixture (plus an uncoupled third package and one
    /// external stub), every edge planted by hand:
    ///
    /// - alpha: `Car` (drive, park), `Engine` (start, stop), free `boot`
    /// - beta: `Wheel` (spin), variable `LIMIT`
    /// - gamma: `Solo` (idle), no edges at all
    ///
    /// Edges: drive→start, drive→spin, park→stop, start→boot,
    /// boot→spin, boot→external::std::fmt (calls); drive reads LIMIT;
    /// start writes LIMIT; drive and park both write the module
    /// variable ODOMETER — their one shared recorded access.
    fn fixture() -> Graph {
        let nodes = vec![
            node("alpha", NodeKind::Package, None, &[]),
            node("beta", NodeKind::Package, None, &[]),
            node("gamma", NodeKind::Package, None, &[]),
            node("alpha::a::Engine", NodeKind::Class, None, &[]),
            node("alpha::a::Engine::start", NodeKind::Method, Some(2), &[]),
            node("alpha::a::Engine::stop", NodeKind::Method, Some(0), &[]),
            node("alpha::a::Car", NodeKind::Class, None, &[]),
            node("alpha::a::Car::drive", NodeKind::Method, Some(1), &["LIMIT"]),
            node("alpha::a::Car::park", NodeKind::Method, Some(3), &[]),
            node("alpha::a::boot", NodeKind::Function, Some(0), &[]),
            node("alpha::a::ODOMETER", NodeKind::Variable, None, &[]),
            node("beta::b::Wheel", NodeKind::Class, None, &[]),
            node("beta::b::Wheel::spin", NodeKind::Method, Some(0), &[]),
            node("beta::b::LIMIT", NodeKind::Variable, None, &[]),
            node("gamma::g::Solo", NodeKind::Class, None, &[]),
            node("gamma::g::Solo::idle", NodeKind::Method, Some(1), &[]),
            node("external::std::fmt", NodeKind::External, None, &[]),
        ];
        let edges = vec![
            edge("alpha::a::Car::drive", "alpha::a::Engine::start", EdgeKind::Calls),
            edge("alpha::a::Car::drive", "beta::b::Wheel::spin", EdgeKind::Calls),
            edge("alpha::a::Car::park", "alpha::a::Engine::stop", EdgeKind::Calls),
            edge("alpha::a::Engine::start", "alpha::a::boot", EdgeKind::Calls),
            edge("alpha::a::boot", "beta::b::Wheel::spin", EdgeKind::Calls),
            edge("alpha::a::boot", "external::std::fmt", EdgeKind::Calls),
            edge("alpha::a::Car::drive", "beta::b::LIMIT", EdgeKind::Reads),
            edge("alpha::a::Engine::start", "beta::b::LIMIT", EdgeKind::Writes),
            edge("alpha::a::Car::drive", "alpha::a::ODOMETER", EdgeKind::Writes),
            edge("alpha::a::Car::park", "alpha::a::ODOMETER", EdgeKind::Writes),
        ];
        Snapshot {
            schema_version: crate::snapshot::SCHEMA_VERSION.to_string(),
            nodes,
            edges,
            audit_head: String::new(),
            audit_log: Vec::new(),
        }
        .to_graph()
    }

    fn type_metrics<'a>(m: &'a GraphMetrics, id: &str) -> &'a TypeMetrics {
        m.types.iter().find(|t| t.id == id).expect("type measured")
    }

    fn package_metrics<'a>(m: &'a GraphMetrics, id: &str) -> &'a PackageMetrics {
        m.packages
            .iter()
            .find(|p| p.id == id)
            .expect("package measured")
    }

    #[test]
    fn type_metrics_hand_verified() {
        let g = fixture();
        let m = g.metrics();
        assert_eq!(m.types.len(), 4, "four types");

        let car = type_metrics(&m, "alpha::a::Car");
        assert_eq!(car.method_count, 2);
        assert_eq!(car.cbo, 2, "coupled to Engine (calls) and Wheel (calls)");
        assert_eq!(car.rfc, 5, "2 own methods + start, spin, stop called");
        assert_eq!(car.atfd, 5, "start, spin, stop, LIMIT, ODOMETER accessed abroad");
        assert_eq!(car.wmc, Some(6), "v(drive)=2 + v(park)=4");
        assert_eq!(car.lcom_sharing_pairs, Some(1), "both write ODOMETER");
        assert_eq!(car.lcom_non_sharing_pairs, Some(0));
        assert_eq!(car.tcc, Some(1.0));

        let engine = type_metrics(&m, "alpha::a::Engine");
        assert_eq!(engine.method_count, 2);
        assert_eq!(engine.cbo, 1, "coupled to Car only; boot is a free function");
        assert_eq!(engine.rfc, 2, "own methods only — boot is not a method");
        assert_eq!(engine.atfd, 1, "LIMIT written; boot is a function, not data/method access");
        assert_eq!(engine.wmc, Some(4), "v(start)=3 + v(stop)=1");
        assert_eq!(engine.lcom_sharing_pairs, Some(0));
        assert_eq!(engine.lcom_non_sharing_pairs, Some(1));
        assert_eq!(engine.tcc, Some(0.0), "measured zero cohesion, not absent");

        let wheel = type_metrics(&m, "beta::b::Wheel");
        assert_eq!(wheel.method_count, 1);
        assert_eq!(wheel.cbo, 1, "Car calls spin");
        assert_eq!(wheel.rfc, 1);
        assert_eq!(wheel.atfd, 0);
        assert_eq!(wheel.wmc, Some(1));
        assert_eq!(wheel.lcom_sharing_pairs, None, "one method: cohesion not measured");
        assert_eq!(wheel.tcc, None);

        let solo = type_metrics(&m, "gamma::g::Solo");
        assert_eq!(solo.cbo, 0);
        assert_eq!(solo.wmc, Some(2));
        assert_eq!(solo.tcc, None);
    }

    #[test]
    fn package_metrics_hand_verified() {
        let g = fixture();
        let m = g.metrics();
        assert_eq!(m.packages.len(), 3, "three packages; the stub is nobody's");

        // Distinct node-to-node dependency pairs alpha -> beta:
        // (drive, spin), (boot, spin), (drive, LIMIT), (start, LIMIT).
        let alpha = package_metrics(&m, "alpha");
        assert_eq!(alpha.ca, 0);
        assert_eq!(alpha.ce, 4);
        assert_eq!(alpha.external_ce, 1, "(boot, external::std::fmt), named separately");
        assert_eq!(alpha.instability, Some(1.0));

        let beta = package_metrics(&m, "beta");
        assert_eq!(beta.ca, 4);
        assert_eq!(beta.ce, 0);
        assert_eq!(beta.external_ce, 0);
        assert_eq!(beta.instability, Some(0.0));

        let gamma = package_metrics(&m, "gamma");
        assert_eq!(gamma.ca, 0);
        assert_eq!(gamma.ce, 0);
        assert_eq!(gamma.external_ce, 0);
        assert_eq!(gamma.instability, None, "no coupling measured: absent, not zero");

        // Cross-check (the A2 pattern): the pair counts equal a raw
        // recount over the graph's edge list.
        let mut raw_pairs: BTreeSet<(&str, &str)> = BTreeSet::new();
        for e in g.edges() {
            if e.kind == EdgeKind::Contains {
                continue;
            }
            if let (Some(a), Some(b)) = (package_of(&e.from), package_of(&e.to)) {
                if a != b {
                    raw_pairs.insert((e.from.as_str(), e.to.as_str()));
                }
            }
        }
        assert_eq!(raw_pairs.len(), 4, "the recount agrees with Ce/Ca");
    }

    #[test]
    fn wmc_absent_when_any_method_unmeasured() {
        let mut snap_graph = fixture();
        // Rebuild with stop unmeasured: WMC(Engine) must go absent.
        let nodes: Vec<Node> = snap_graph
            .nodes()
            .map(|n| {
                let mut n = n.clone();
                if n.id == "alpha::a::Engine::stop" {
                    n.decisions = None;
                }
                n
            })
            .collect();
        let edges: Vec<Edge> = snap_graph.edges().cloned().collect();
        snap_graph = Snapshot {
            schema_version: crate::snapshot::SCHEMA_VERSION.to_string(),
            nodes,
            edges,
            audit_head: String::new(),
            audit_log: Vec::new(),
        }
        .to_graph();
        let m = snap_graph.metrics();
        let engine = type_metrics(&m, "alpha::a::Engine");
        assert_eq!(engine.wmc, None, "one unmeasured method: no sum is reported");
        let car = type_metrics(&m, "alpha::a::Car");
        assert_eq!(car.wmc, Some(6), "other types unaffected");
    }
}
