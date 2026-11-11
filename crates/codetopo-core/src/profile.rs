// Copyright (C) 2026 2351485 ALBERTA INC.
// SPDX-License-Identifier: AGPL-3.0-or-later
// Part of codetopo — Owned by 2351485 ALBERTA INC. (https://nrupalakolkar.com) · Built with Muse by Meta (https://muse.ai/join).

//! Domain profiles — the dictionary of a tenant domain as data.
//!
//! A profile declares everything the pre-2.0 core hard-coded for code:
//! node kinds, edge kinds and their attribute allow-lists, the three
//! edge sets (dependency / breakage / closure), the boundary kind and
//! its id namespace, measured fields, the tier rule, provenance
//! classes, and the domain's sibling measures. The core machinery —
//! builder, queries, projection, snapshot envelope — reads the loaded
//! profile instead of matching on a closed code dictionary.
//!
//! The founding profile is CODE itself: `profiles/code.profile.json`
//! at the repository root, a mechanical transcription of
//! CODE_SCHEMA v1.2, embedded here so every build carries it. The
//! CODE-Profile Regression Contract proves the transcription changed
//! no answer.
//!
//! Validation is loud (§8's posture applied to profiles): a profile
//! with a wrong format string, a core-major mismatch, a non-total
//! tier rule, an edge set that is not a subset of the dependency set,
//! or an edge kind without a provenance class is refused at load,
//! never partially applied (founding spec §4, R1).

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt;
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

use crate::schema::{EdgeKind, NodeKind, Tier};

/// The profile file format this core reads.
pub const PROFILE_FORMAT: &str = "codetopo.profile/1";

/// The embedded CODE profile (profile #1).
pub const CODE_PROFILE_JSON: &str = include_str!("../../../profiles/code.profile.json");

/// Provenance classes (founding spec §4.1), ordered by strength:
/// `extracted_truth` > `sourced_fact` > `estimate` > `inference` >
/// `declared_claim`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProvenanceClass {
    ExtractedTruth,
    SourcedFact,
    Estimate,
    Inference,
    DeclaredClaim,
}

impl ProvenanceClass {
    pub fn as_str(self) -> &'static str {
        match self {
            ProvenanceClass::ExtractedTruth => "extracted_truth",
            ProvenanceClass::SourcedFact => "sourced_fact",
            ProvenanceClass::Estimate => "estimate",
            ProvenanceClass::Inference => "inference",
            ProvenanceClass::DeclaredClaim => "declared_claim",
        }
    }

    /// The provenance attributes an edge of this class must carry
    /// (founding spec §4.1's table). These are core law — the classes
    /// and their mandatory attrs are the mechanism, not profile data.
    pub fn mandatory_attrs(self) -> &'static [&'static str] {
        match self {
            ProvenanceClass::ExtractedTruth => &[],
            ProvenanceClass::SourcedFact => &["source", "as_of"],
            ProvenanceClass::Estimate => &["method", "window", "error"],
            ProvenanceClass::Inference => &["dataset", "method", "support"],
            ProvenanceClass::DeclaredClaim => &["claimant"],
        }
    }
}

impl fmt::Display for ProvenanceClass {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One node kind of a profile's dictionary.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NodeKindDef {
    pub name: String,
    pub gloss: String,
    /// "root" | "intermediate" | "leaf" | "none".
    pub containment_role: String,
    /// Tie-break rank when a name resolves to several node kinds
    /// (the builder's resolve/search ordering): lower sorts first.
    /// CODE's table: symbol kinds 0, container kinds 1, boundary
    /// and doc kinds 2 — recorded as data because the ordering is
    /// part of the dictionary's measured behaviour.
    #[serde(default = "default_resolve_rank")]
    pub resolve_rank: u8,
    pub tier: Tier,
}

fn default_resolve_rank() -> u8 {
    1
}

/// The attribute row of one edge kind.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AttrDef {
    /// "string" | "number" | "boolean" | "string[]" | "number[]".
    #[serde(rename = "type")]
    pub attr_type: String,
    pub required: bool,
}

/// One edge kind of a profile's dictionary.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EdgeKindDef {
    pub name: String,
    pub gloss: String,
    pub from_kinds: Vec<String>,
    pub to_kinds: Vec<String>,
    #[serde(default)]
    pub attrs: BTreeMap<String, AttrDef>,
    pub provenance_class: ProvenanceClass,
    /// Attr names the profile additionally requires as provenance
    /// evidence for this kind (beyond the class's mandatory attrs).
    #[serde(default)]
    pub provenance_attrs: Vec<String>,
    /// "live" | "restore_only" (founding spec D2).
    pub emission: String,
}

/// One measured field of a profile.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MeasureFieldDef {
    pub name: String,
    pub on_kinds: Vec<String>,
    /// "integer" | "number" | "string".
    #[serde(rename = "type")]
    pub field_type: String,
    /// The normative extraction rule, stated in the profile itself.
    pub extraction_rule: String,
    /// The only absence semantics: "not_measured".
    pub absence: String,
}

/// An aggregate kind a sibling ranges over (the profile's analogue of
/// §9's *type* / *package*).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AggregateKindDef {
    pub name: String,
    pub node_kinds: Vec<String>,
    /// "id_prefix_one_segment" | "first_id_segment" | "containment" |
    /// "declared_rule".
    pub membership: String,
}

/// A measure-sibling registration (founding spec §1.4). Registration
/// is declarative; implementation is tenant code behind the registry.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SiblingDef {
    /// Namespaced: `<profile_id>.<name>`; core ids are reserved.
    pub measure_id: String,
    pub name: String,
    pub title: String,
    pub citation: String,
    /// "node" | "aggregate" | "pair" | "graph".
    pub scope: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aggregate_kind: Option<AggregateKindDef>,
    #[serde(default)]
    pub inputs: SiblingInputs,
    /// The normative definition, published beside every result.
    pub method_text: String,
    /// "graph" | "statistics".
    pub layer: String,
    /// Pointer into the profile's conformance corpus. No oracle, no
    /// registration — enforced at load.
    pub oracle: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SiblingInputs {
    #[serde(default)]
    pub measured_fields: Vec<String>,
    #[serde(default)]
    pub edge_sets: Vec<String>,
}

/// One well-formedness constraint: a §5-analogue template mapped to a
/// core diagnostic slot.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WellFormednessDef {
    pub name: String,
    pub rule: String,
    /// The core diagnostic slot: "multi_definition" | "cycle" |
    /// "unresolved_call" | "unknown_symbol" | "invalid_attribute" |
    /// "missing_provenance".
    pub diagnostic: String,
    #[serde(default)]
    pub resolvable_kinds: Vec<String>,
}

/// The profile's §1.6 oracle declaration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConformanceDef {
    pub oracle: String,
    pub corpus: Vec<String>,
    pub tolerances: String,
}

/// The profile's history parameterisation (founding spec D5).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HistoryDef {
    /// The node kind transactions range over (CODE: `file`).
    pub transaction_unit_kind: String,
    /// The sibling the hotspot factor sums, or absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hotspot_factor_sibling: Option<String>,
    #[serde(default)]
    pub methods: BTreeMap<String, String>,
}

/// A loaded, validated domain profile.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DomainProfile {
    pub format: String,
    pub profile_id: String,
    pub profile_version: String,
    pub title: String,
    pub core_compat: CoreCompat,
    pub admission: Admission,
    pub node_kinds: Vec<NodeKindDef>,
    pub edge_kinds: Vec<EdgeKindDef>,
    pub containment: ContainmentDef,
    pub edge_sets: EdgeSets,
    pub boundary: BoundaryDef,
    pub measured_fields: Vec<MeasureFieldDef>,
    pub id_grammar: serde_json::Value,
    pub location: serde_json::Value,
    /// Total kind→tier map; the "default" key is required.
    pub tier_rule: BTreeMap<String, Tier>,
    pub well_formedness: Vec<WellFormednessDef>,
    #[serde(default)]
    pub siblings: Vec<SiblingDef>,
    pub conformance: ConformanceDef,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub history: Option<HistoryDef>,

    #[serde(skip)]
    node_index: HashMap<String, usize>,
    #[serde(skip)]
    edge_index: HashMap<String, usize>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CoreCompat {
    pub core_schema_major: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Admission {
    pub provable: bool,
    pub logic_structure: bool,
    pub connectivity: bool,
    pub extractable_truths: bool,
    pub admitted_classes: Vec<ProvenanceClass>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ContainmentDef {
    pub edge_kind: String,
    pub root_kind: String,
    pub forest: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EdgeSets {
    pub dependency: Vec<String>,
    pub breakage: Vec<String>,
    pub closure: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BoundaryDef {
    pub kind: String,
    pub id_namespace: String,
}

impl DomainProfile {
    /// Parse and validate a profile file. Refusals are loud and name
    /// the broken rule (founding spec §1.1, §4 R1).
    pub fn from_json(text: &str) -> Result<Self, String> {
        let mut profile: DomainProfile =
            serde_json::from_str(text).map_err(|e| format!("invalid profile JSON: {e}"))?;
        profile.rebuild_indexes();
        profile.validate()?;
        Ok(profile)
    }

    fn rebuild_indexes(&mut self) {
        self.node_index = self
            .node_kinds
            .iter()
            .enumerate()
            .map(|(i, k)| (k.name.clone(), i))
            .collect();
        self.edge_index = self
            .edge_kinds
            .iter()
            .enumerate()
            .map(|(i, k)| (k.name.clone(), i))
            .collect();
    }

    /// The founding validation battery (§1.1 + §4 R1): format string,
    /// core-major compatibility, unique kind names, tier totality
    /// (every node kind maps, and "default" exists), edge-set
    /// containment (B ⊆ D and C ⊆ D), every edge-set member a declared
    /// kind, boundary/containment kinds declared, a provenance class
    /// on every edge kind (guaranteed by the type — a missing or
    /// unknown class fails JSON parsing), and every sibling carrying
    /// an oracle and a namespaced id.
    pub fn validate(&self) -> Result<(), String> {
        if self.format != PROFILE_FORMAT {
            return Err(format!(
                "unsupported profile format {:?} (this build reads {PROFILE_FORMAT:?})",
                self.format
            ));
        }
        let core_major = crate::snapshot::SCHEMA_VERSION
            .split('.')
            .next()
            .and_then(|s| s.parse::<u32>().ok())
            .expect("own schema version parses");
        if self.core_compat.core_schema_major != core_major {
            return Err(format!(
                "profile {:?} targets core schema major {} but this build is major {core_major}",
                self.profile_id, self.core_compat.core_schema_major
            ));
        }
        let mut seen = BTreeSet::new();
        for k in &self.node_kinds {
            if !seen.insert(k.name.as_str()) {
                return Err(format!("duplicate node kind {:?}", k.name));
            }
            if !self.tier_rule.contains_key(&k.name) && !self.tier_rule.contains_key("default") {
                return Err(format!(
                    "tier rule is not total: node kind {:?} has no tier and no default",
                    k.name
                ));
            }
            if k.tier != self.tier_for_name(&k.name) {
                return Err(format!(
                    "node kind {:?} declares tier {:?} but the tier rule says {:?}",
                    k.name, k.tier, self.tier_for_name(&k.name)
                ));
            }
        }
        if !self.tier_rule.contains_key("default") {
            return Err("tier rule must carry a \"default\" entry".to_string());
        }
        let mut eseen = BTreeSet::new();
        for k in &self.edge_kinds {
            if !eseen.insert(k.name.as_str()) {
                return Err(format!("duplicate edge kind {:?}", k.name));
            }
            for kind in k.from_kinds.iter().chain(k.to_kinds.iter()) {
                if kind != "*" && !self.has_node_kind_name(kind) {
                    return Err(format!(
                        "edge kind {:?} names undeclared node kind {kind:?}",
                        k.name
                    ));
                }
            }
            if !self.admission.admitted_classes.contains(&k.provenance_class) {
                return Err(format!(
                    "edge kind {:?} declares provenance class {} which the profile does not admit",
                    k.name, k.provenance_class
                ));
            }
        }
        let dep: BTreeSet<&str> =
            self.edge_sets.dependency.iter().map(|s| s.as_str()).collect();
        for (set_name, set) in [
            ("dependency", &self.edge_sets.dependency),
            ("breakage", &self.edge_sets.breakage),
            ("closure", &self.edge_sets.closure),
        ] {
            for name in set {
                if !self.has_edge_kind_name(name) {
                    return Err(format!(
                        "edge set {set_name} names undeclared edge kind {name:?}"
                    ));
                }
                if set_name != "dependency" && !dep.contains(name.as_str()) {
                    return Err(format!(
                        "edge set {set_name} is not a subset of the dependency set: {name:?}"
                    ));
                }
            }
        }
        if !self.has_node_kind_name(&self.boundary.kind) {
            return Err(format!(
                "boundary kind {:?} is not a declared node kind",
                self.boundary.kind
            ));
        }
        if !self.has_edge_kind_name(&self.containment.edge_kind) {
            return Err(format!(
                "containment edge kind {:?} is not a declared edge kind",
                self.containment.edge_kind
            ));
        }
        if !self.has_node_kind_name(&self.containment.root_kind) {
            return Err(format!(
                "containment root kind {:?} is not a declared node kind",
                self.containment.root_kind
            ));
        }
        for f in &self.measured_fields {
            if f.absence != "not_measured" {
                return Err(format!(
                    "measured field {:?} declares absence {:?}; the only absence semantics is \"not_measured\"",
                    f.name, f.absence
                ));
            }
            for kind in &f.on_kinds {
                if !self.has_node_kind_name(kind) {
                    return Err(format!(
                        "measured field {:?} names undeclared node kind {kind:?}",
                        f.name
                    ));
                }
            }
        }
        for s in &self.siblings {
            if s.oracle.trim().is_empty() {
                return Err(format!(
                    "sibling {:?} has no oracle — no oracle, no registration",
                    s.measure_id
                ));
            }
            let prefix = format!("{}.", self.profile_id);
            if !s.measure_id.starts_with(&prefix) && !s.measure_id.starts_with("core.") {
                return Err(format!(
                    "sibling id {:?} is not namespaced under {:?}",
                    s.measure_id, self.profile_id
                ));
            }
        }
        Ok(())
    }

    /// The CODE profile (profile #1), parsed once from the embedded
    /// transcription of CODE_SCHEMA v1.2.
    pub fn code() -> std::sync::Arc<DomainProfile> {
        static CODE: OnceLock<std::sync::Arc<DomainProfile>> = OnceLock::new();
        CODE.get_or_init(|| {
            std::sync::Arc::new(
                DomainProfile::from_json(CODE_PROFILE_JSON)
                    .expect("the embedded CODE profile validates"),
            )
        })
        .clone()
    }

    pub fn node_kind_def(&self, kind: &NodeKind) -> Option<&NodeKindDef> {
        self.node_index.get(kind.as_str()).map(|&i| &self.node_kinds[i])
    }

    pub fn edge_kind_def(&self, kind: &EdgeKind) -> Option<&EdgeKindDef> {
        self.edge_index.get(kind.as_str()).map(|&i| &self.edge_kinds[i])
    }

    pub fn has_node_kind(&self, kind: &NodeKind) -> bool {
        self.node_index.contains_key(kind.as_str())
    }

    pub fn has_node_kind_name(&self, name: &str) -> bool {
        self.node_index.contains_key(name)
    }

    pub fn has_edge_kind_name(&self, name: &str) -> bool {
        self.edge_index.contains_key(name)
    }

    /// Node kinds in declaration order — the tally order the surfaces
    /// report.
    pub fn node_kinds_in_order(&self) -> Vec<NodeKind> {
        self.node_kinds.iter().map(|k| NodeKind::new(&k.name)).collect()
    }

    /// Edge kinds in declaration order.
    pub fn edge_kinds_in_order(&self) -> Vec<EdgeKind> {
        self.edge_kinds.iter().map(|k| EdgeKind::new(&k.name)).collect()
    }

    /// The tier the profile assigns a node kind (tier_rule lookup;
    /// total by validation).
    pub fn tier_for(&self, kind: &NodeKind) -> Tier {
        self.tier_for_name(kind.as_str())
    }

    /// A node kind's resolve tie-break rank (profile data).
    pub fn resolve_rank(&self, kind: &NodeKind) -> u8 {
        self.node_kind_def(kind).map(|d| d.resolve_rank).unwrap_or(2)
    }

    pub fn tier_for_name(&self, name: &str) -> Tier {
        self.tier_rule
            .get(name)
            .copied()
            .unwrap_or_else(|| self.tier_rule["default"])
    }

    /// The attribute allow-list of an edge kind (founding spec §1.2
    /// part 2 — the profile table replaces the hard-coded match).
    /// Unknown kinds allow nothing.
    pub fn allowed_attrs(&self, kind: &EdgeKind) -> Vec<&str> {
        match self.edge_kind_def(kind) {
            Some(def) => def.attrs.keys().map(|s| s.as_str()).collect(),
            None => Vec::new(),
        }
    }

    /// Declaration rank of an edge kind: its position in the profile.
    /// Unknown kinds sort after every declared kind, by name — a
    /// deterministic fallback for restored legacy data.
    pub fn edge_rank(&self, kind: &EdgeKind) -> (usize, String) {
        match self.edge_index.get(kind.as_str()) {
            Some(&i) => (i, String::new()),
            None => (usize::MAX, kind.as_str().to_string()),
        }
    }

    pub fn is_dependency(&self, kind: &EdgeKind) -> bool {
        self.edge_sets.dependency.iter().any(|n| n == kind.as_str())
    }

    pub fn is_breakage(&self, kind: &EdgeKind) -> bool {
        self.edge_sets.breakage.iter().any(|n| n == kind.as_str())
    }

    pub fn is_closure(&self, kind: &EdgeKind) -> bool {
        self.edge_sets.closure.iter().any(|n| n == kind.as_str())
    }

    pub fn is_containment(&self, kind: &EdgeKind) -> bool {
        kind.as_str() == self.containment.edge_kind
    }

    /// The boundary test (founding spec D8): the boundary *kind* or
    /// the boundary id-namespace prefix — both, as the implementation
    /// has always done, now read from the profile.
    pub fn is_boundary(&self, kind: &NodeKind, id: &str) -> bool {
        kind.as_str() == self.boundary.kind || id.starts_with(&self.boundary.id_namespace)
    }

    /// Package membership (§9's first-segment rule, as profile data):
    /// the first `::` segment of the id, or `None` for boundary stubs
    /// — they are nobody's package.
    pub fn package_of<'a>(&self, id: &'a str) -> Option<&'a str> {
        if id.starts_with(&self.boundary.id_namespace) {
            None
        } else {
            Some(id.split("::").next().unwrap_or(id))
        }
    }

    /// The mandatory provenance attrs of an edge kind: its class's
    /// core-mandated attrs plus the profile's per-kind additions.
    pub fn provenance_required_attrs(&self, kind: &EdgeKind) -> Vec<&str> {
        match self.edge_kind_def(kind) {
            Some(def) => {
                let mut out: Vec<&str> = def.provenance_class.mandatory_attrs().to_vec();
                for a in &def.provenance_attrs {
                    if !out.contains(&a.as_str()) {
                        out.push(a.as_str());
                    }
                }
                out
            }
            None => Vec::new(),
        }
    }

    /// The provenance class of an edge kind, if declared.
    pub fn provenance_class(&self, kind: &EdgeKind) -> Option<ProvenanceClass> {
        self.edge_kind_def(kind).map(|d| d.provenance_class)
    }

    /// A registered sibling by its short name (`"cbo"`) or full id
    /// (`"code.cbo"`).
    pub fn sibling(&self, name: &str) -> Option<&SiblingDef> {
        self.siblings
            .iter()
            .find(|s| s.name == name || s.measure_id == name)
    }

    /// The method texts of the registered siblings, keyed by short
    /// name — the registry form of the old `METRIC_METHODS` constant.
    pub fn metric_methods(&self) -> Vec<(String, String)> {
        self.siblings
            .iter()
            .map(|s| (s.name.clone(), s.method_text.clone()))
            .collect()
    }

    /// The method text of one core measure: fan-in / fan-out are core
    /// counts over the dependency set (registry law 3 — they need no
    /// registration, but their texts still come from one place), and
    /// the centrality family keeps its §9 method texts here as the
    /// profile-independent core definitions.
    pub fn core_method(&self, name: &str) -> Option<String> {
        let dep = self.edge_sets.dependency.join(", ");
        let non_dep = self.containment.edge_kind.clone();
        match name {
            "fan_in" => Some(format!(
                "fan-in (Henry & Kafura 1981): the number of incoming dependency edges \
                 (dependency set: {dep}; the containment kind {non_dep} is hierarchy, \
                 not dependency); boundary stub nodes are not ranked"
            )),
            "fan_out" => Some(format!(
                "fan-out (Henry & Kafura 1981): the number of outgoing dependency edges \
                 (dependency set: {dep}; the containment kind {non_dep} is hierarchy, \
                 not dependency); boundary stub nodes are not ranked"
            )),
            "blast_radius" => Some(format!(
                "blast radius: the number of transitive dependents over the breakage \
                 edge set ({}) of profile {:?}; boundary stub nodes are not ranked",
                self.edge_sets.breakage.join(", "),
                self.profile_id
            )),
            "pagerank" => Some(
                "PageRank (Brin & Page 1998): the stationary distribution of a random \
                 walk with teleportation (damping 0.85, power iteration from the \
                 uniform vector, max 100 iterations, stopping when the L1 step falls \
                 below n x 1e-6 — the 1e-6 is the stopping rule, not an accuracy \
                 bound) over the profile's dependency projection as a simple graph \
                 (parallel edges collapse, self-loops dropped, boundary stubs \
                 excluded); every node in the projection is ranked"
                    .to_string(),
            ),
            "betweenness" => Some(format!(
                "betweenness centrality (Freeman 1977; Brandes 2001): the fraction of \
                 all-pairs shortest paths through the node, endpoints excluded, \
                 normalized, exact (no sampling), over the profile's dependency \
                 projection as a simple graph; every node in the projection is ranked \
                 and 0.0 is a measured value; graphs over {} nodes are declined, with \
                 the ceiling as the stated reason",
                crate::centrality::BETWEENNESS_NODE_CEILING
            )),
            "closeness" => Some(
                "closeness centrality, Wasserman-Faust form (Wasserman & Faust 1994): \
                 (reachable-1)/(n-1) x (reachable-1)/(sum of distances), unweighted \
                 BFS distances over the profile's dependency projection as a simple \
                 graph; every node in the projection is ranked and a node that \
                 nothing reaches measures 0.0"
                    .to_string(),
            ),
            "eigenvector" => Some(
                "eigenvector centrality (Bonacich 1972): the dominant eigenvector of \
                 the adjacency matrix of the profile's dependency projection, \
                 L2-normalized, sign fixed by positive sum, power iteration tolerance \
                 1e-6, max 100 iterations; nodes outside the dominant strongly \
                 connected region can measure 0 (the method's result — Katz does \
                 not collapse); non-convergence is reported as an absence, never a \
                 partial vector"
                    .to_string(),
            ),
            "katz" => Some(
                "Katz centrality (Katz 1953): all walks counted, attenuated by alpha \
                 = 0.1 per step plus beta = 1.0 bias, over the profile's dependency \
                 projection as a simple graph, power iteration tolerance 1e-6, max \
                 100 iterations, L2-normalized; non-convergence is reported as an \
                 absence, never a partial vector"
                    .to_string(),
            ),
            "communities" => Some(format!(
                "Leiden community detection (Traag, Waltman & van Eck 2019), \
                 modularity quality function (Newman & Girvan 2004), resolution \
                 gamma = 1.0, fixed seed {}, over the undirected projection of the \
                 profile's dependency set weighted by recorded-edge multiplicity \
                 (boundary stubs excluded, self-loops dropped); community ids are \
                 assigned by smallest member id, ascending; an isolated node is a \
                 singleton community; modularity Q is computed by codetopo from the \
                 same projection and is absent when the graph has no edges (Q is \
                 undefined at m = 0)",
                crate::communities::LEIDEN_SEED
            )),
            "topo_sort" => Some(
                "topological order of the SCC-condensed dependency DAG (Kahn's \
                 algorithm; strongly connected components by Tarjan, the \
                 condensation the layered layout already uses): groups are emitted \
                 in dependency order — every dependency of a group appears in an \
                 earlier group — members are sorted by id, and ready groups are \
                 taken by smallest member id; a multi-node group is a cycle and is \
                 flagged cyclic, never disguised as an order; self-loops are \
                 dropped by the projection, so a self-referencing node appears as \
                 a singleton group"
                    .to_string(),
            ),
            _ => None,
        }
    }

    /// The graph class of a set of edges under this profile (founding
    /// spec §4.3): `truths` iff every edge's class is extracted_truth
    /// or sourced_fact; otherwise `claims-bearing`, with the counts.
    pub fn graph_class_of<'a>(
        &self,
        kinds: impl Iterator<Item = &'a EdgeKind>,
    ) -> (String, BTreeMap<String, usize>) {
        let mut summary: BTreeMap<String, usize> = BTreeMap::new();
        let mut truths = true;
        for k in kinds {
            let class = self
                .provenance_class(k)
                .map(|c| c.as_str().to_string())
                .unwrap_or_else(|| "undeclared".to_string());
            if class != "extracted_truth" && class != "sourced_fact" {
                truths = false;
            }
            *summary.entry(class).or_insert(0) += 1;
        }
        (
            if truths { "truths".to_string() } else { "claims-bearing".to_string() },
            summary,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_code_profile_validates() {
        let p = DomainProfile::code();
        assert_eq!(p.profile_id, "code");
        assert_eq!(p.profile_version, "1.0.0");
        assert_eq!(p.node_kinds.len(), 10);
        assert_eq!(p.edge_kinds.len(), 9);
        assert_eq!(p.siblings.len(), 12);
        // D2/D3: references is restore-only and allows only `line`.
        let refs = p.edge_kind_def(&EdgeKind::References).expect("references declared");
        assert_eq!(refs.emission, "restore_only");
        assert_eq!(p.allowed_attrs(&EdgeKind::References), vec!["line"]);
        assert_eq!(p.allowed_attrs(&EdgeKind::Reads), vec!["line"]);
        assert_eq!(p.allowed_attrs(&EdgeKind::Writes), vec!["line"]);
        // D1: the three sets are distinct and correctly contained.
        assert!(p.is_dependency(&EdgeKind::References));
        assert!(!p.is_breakage(&EdgeKind::Imports));
        assert!(p.is_closure(&EdgeKind::Imports));
        assert!(!p.is_closure(&EdgeKind::Defines));
        // D8: the boundary test reads the profile.
        assert!(p.is_boundary(&NodeKind::External, "anything"));
        assert!(p.is_boundary(&NodeKind::Function, "external::serde"));
        assert!(!p.is_boundary(&NodeKind::Function, "pkg::f"));
        assert_eq!(p.package_of("external::serde"), None);
        assert_eq!(p.package_of("alpha::a::f"), Some("alpha"));
    }

    #[test]
    fn profile_without_a_provenance_class_is_refused() {
        // R1: no class, no profile. Removing the field breaks the
        // grammar itself, so the refusal happens at parse.
        let broken = CODE_PROFILE_JSON.replace(
            "\"provenance_class\": \"extracted_truth\"",
            "\"provenance_class\": \"vibes\"",
        );
        assert!(DomainProfile::from_json(&broken).is_err());
        let missing = CODE_PROFILE_JSON.replace("\"provenance_class\":", "\"provenance_was\":");
        assert!(DomainProfile::from_json(&missing).is_err());
    }

    #[test]
    fn profile_with_a_non_subset_edge_set_is_refused() {
        // Break the subset law semantically (not textually, so the
        // test survives profile-file reformatting): swap the
        // breakage set for one carrying `contains`, which is not in
        // the dependency set.
        let mut value: serde_json::Value =
            serde_json::from_str(CODE_PROFILE_JSON).expect("profile parses");
        value["edge_sets"]["breakage"] =
            serde_json::json!(["calls", "contains"]);
        let broken = serde_json::to_string(&value).expect("reserialize");
        let err = DomainProfile::from_json(&broken).expect_err("B not subset of D must refuse");
        assert!(err.contains("not a subset"), "{err}");
    }
}
