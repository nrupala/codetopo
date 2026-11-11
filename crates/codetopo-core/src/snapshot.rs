// Copyright (C) 2026 2351485 ALBERTA INC.
// SPDX-License-Identifier: AGPL-3.0-or-later
// Part of codetopo — Owned by 2351485 ALBERTA INC. (https://nrupalakolkar.com) · Built with Muse by Meta (https://muse.ai/join).

//! JSON snapshot pathway (CODE_SCHEMA §7: JSON is the universal export
//! pathway). A snapshot is the frozen, portable form of a [`Graph`]: nodes,
//! edges, the schema version the producer emitted, the head of the
//! tamper-evident audit log it was exported under, and — when the producer
//! recorded them — the index-time diagnostics (CODE_SCHEMA §5), so a
//! restored graph reports the same diagnostics as the graph it was
//! exported from.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::diag::Diagnostic;
use crate::graph::Graph;
use crate::profile::DomainProfile;
use crate::schema::{Edge, EdgeKind, Node};

fn default_profile_id() -> String {
    DomainProfile::code().profile_id.clone()
}

fn default_profile_version() -> String {
    DomainProfile::code().profile_version.clone()
}

fn default_graph_class() -> String {
    "truths".to_string()
}

/// Core schema version this implementation emits. 2.0 is the
/// domain-profile move: the dictionary became profile data, and the
/// snapshot gained its `profile_id` / `profile_version` stamps plus
/// the computed `graph_class` / `provenance_summary` (§4.3). 1.x
/// snapshots still read — they name no profile because only one
/// dictionary existed, so they load under the CODE profile by
/// definition (founding spec §1.3) — and their nodes simply carry no
/// profile measures.
pub const SCHEMA_VERSION: &str = "2.0";

/// Portable frozen graph.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Snapshot {
    pub schema_version: String,
    /// The domain profile the graph's dictionary belongs to
    /// (founding spec §1.3). Absent in 1.x snapshots; defaults to
    /// `code`, the only dictionary that existed then.
    #[serde(default = "default_profile_id")]
    pub profile_id: String,
    /// The profile's own version (semver; §8's law applied to
    /// profiles). Absent in 1.x snapshots; defaults to the CODE
    /// profile's founding version.
    #[serde(default = "default_profile_version")]
    pub profile_version: String,
    /// The computed graph class (§4.3): `truths` iff every edge is
    /// extracted_truth or sourced_fact, else `claims-bearing`.
    /// Computed at export, never declared; defaults on read so a
    /// hand-written or 1.x snapshot parses — consumers that need the
    /// stamp recompute it from the graph's profile.
    #[serde(default = "default_graph_class")]
    pub graph_class: String,
    /// Edge counts by provenance class (§4.3), stamped beside
    /// `graph_class` at export.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub provenance_summary: std::collections::BTreeMap<String, usize>,
    #[serde(default)]
    pub nodes: Vec<Node>,
    #[serde(default)]
    pub edges: Vec<Edge>,
    /// Head hash of the tamper-evident audit log at export time. This crate
    /// does not own the log — it just binds the snapshot to it.
    #[serde(default)]
    pub audit_head: String,
    /// The audit-log entries the snapshot was exported under, oldest first.
    /// Restoring a snapshot replays these entries verbatim, so the restored
    /// database binds the same audit head and a snapshot → restore →
    /// snapshot round-trip is byte-identical. `#[serde(default)]` keeps
    /// snapshots written before this field existed readable (they restore
    /// with a fresh genesis log instead).
    #[serde(default)]
    pub audit_log: Vec<SnapshotAuditEntry>,
    /// The index-time diagnostics recorded for the exported graph, in
    /// builder order — the same vector the store persists and the
    /// diagnostics answer serves. Additive like the audit trail: snapshots
    /// written before this field existed parse with an empty vector (they
    /// genuinely carry no recorded diagnostics), and the field is omitted
    /// from the JSON entirely when empty, so a diagnostics-free graph's
    /// snapshot bytes are exactly what earlier versions wrote.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<Diagnostic>,
}

/// One entry of the tamper-evident audit log, as carried by a snapshot.
/// Field-for-field identical to the store crate's `AuditEntry`; defined here
/// so snapshots can transport the chain without core depending on the store
/// crate.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SnapshotAuditEntry {
    pub seq: u64,
    pub ts: String,
    pub op: String,
    pub payload: String,
    pub prev_hash: String,
    pub hash: String,
}

impl Snapshot {
    /// Export a graph. Nodes are stored sorted by id; edges are stored
    /// sorted by `(from, to, kind)` — with `kind` compared as its snake_case
    /// wire string, exactly matching the store's
    /// `ORDER BY from_id, to_id, kind` — so the JSON is deterministic and a
    /// snapshot → restore → snapshot round-trip is byte-identical.
    /// `audit_head` is the hash of the last entry of `audit_log`
    /// (empty string when the log is empty).
    pub fn from_graph(g: &Graph, audit_log: &[SnapshotAuditEntry]) -> Self {
        Self::from_graph_with_diagnostics(g, audit_log, &[])
    }

    /// Export a graph with the diagnostics recorded when it was indexed.
    /// Diagnostics live outside the [`Graph`] (the builder returns them
    /// separately and the store persists them in their own table), so a
    /// producer that has them — an index run, or any store-backed surface —
    /// passes them here; a producer without them exports via
    /// [`Snapshot::from_graph`] and the snapshot carries none.
    pub fn from_graph_with_diagnostics(
        g: &Graph,
        audit_log: &[SnapshotAuditEntry],
        diagnostics: &[Diagnostic],
    ) -> Self {
        let mut edges: Vec<Edge> = g.edges().cloned().collect();
        edges.sort_by(|a, b| {
            a.from
                .cmp(&b.from)
                .then_with(|| a.to.cmp(&b.to))
                .then_with(|| a.kind.as_str().cmp(b.kind.as_str()))
        });
        let profile = g.profile();
        let (graph_class, provenance_summary) =
            profile.graph_class_of(g.edges().map(|e| &e.kind));
        Snapshot {
            schema_version: SCHEMA_VERSION.to_string(),
            profile_id: profile.profile_id.clone(),
            profile_version: profile.profile_version.clone(),
            graph_class,
            provenance_summary,
            nodes: g.nodes().cloned().collect(),
            edges,
            audit_head: audit_log.last().map(|e| e.hash.clone()).unwrap_or_default(),
            audit_log: audit_log.to_vec(),
            diagnostics: diagnostics.to_vec(),
        }
    }

    /// The major component of a `MAJOR.MINOR...` schema version string
    /// (`"1.0-draft"` → `Some(1)`). `None` when the string does not start
    /// with a number — an unparseable version is never silently accepted.
    pub fn schema_major(version: &str) -> Option<u32> {
        version.split('.').next()?.parse().ok()
    }

    /// Enforce §8: a consumer rejects a snapshot whose core schema
    /// major it does not speak — loudly, never by guessing. This
    /// build speaks major 2 (profile-driven) and major 1 (the
    /// pre-profile line, which loads under the CODE profile by
    /// definition — founding spec §1.3). Same-major minor drift is
    /// tolerated, per the §8 rule.
    pub fn check_schema_version(&self) -> Result<(), String> {
        match Self::schema_major(&self.schema_version) {
            Some(1) | Some(2) => Ok(()),
            _ => Err(format!(
                "unsupported snapshot schema_version {:?} (this build supports majors 1 and 2)",
                self.schema_version
            )),
        }
    }

    pub fn to_json_pretty(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }

    pub fn from_json(s: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(s)
    }

    /// Rebuild the graph directly from snapshot data — no re-validation, no
    /// re-resolution. Adjacency maps are rebuilt from the stored edges and
    /// sorted so query determinism holds for deserialized graphs exactly as
    /// for freshly built ones.
    pub fn to_graph(&self) -> Graph {
        self.to_graph_checked()
            .expect("snapshot names a loadable profile")
    }

    /// [`Snapshot::to_graph`] with the refusal surfaced: a snapshot
    /// naming a profile this build cannot load is an error, never a
    /// silent read under the CODE dictionary.
    pub fn to_graph_checked(&self) -> Result<Graph, String> {
        let mut nodes: HashMap<String, Node> = HashMap::new();
        for n in &self.nodes {
            nodes.insert(n.id.clone(), n.clone());
        }
        let mut children: HashMap<String, Vec<(String, EdgeKind)>> = HashMap::new();
        let mut parents: HashMap<String, Vec<(String, EdgeKind)>> = HashMap::new();
        for e in &self.edges {
            children.entry(e.from.clone()).or_default().push((e.to.clone(), e.kind));
            parents.entry(e.to.clone()).or_default().push((e.from.clone(), e.kind));
        }
        let graph = Graph::from_parts(nodes, self.edges.clone(), children, parents);
        if self.profile_id == DomainProfile::code().profile_id {
            Ok(graph)
        } else {
            Err(format!(
                "snapshot names profile {:?} which this build cannot load (only {:?} is embedded)",
                self.profile_id,
                DomainProfile::code().profile_id
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// B0.2 / CODE_SCHEMA §8: a snapshot written before schema 1.1 —
    /// version "1.0-draft", nodes with none of the measured fields —
    /// parses, passes the version check, and reads every measurement
    /// as absent (never zero).
    #[test]
    fn schema_1_0_snapshot_reads_with_absent_measurements() {
        let text = r#"{
          "schema_version": "1.0-draft",
          "nodes": [
            {"id": "pkg::m::f", "kind": "function", "label": "f",
             "loc": {"file": "src/m.rs", "line": 3}, "tier": "optional",
             "defines": [], "refs": []}
          ],
          "edges": [],
          "audit_head": "",
          "audit_log": []
        }"#;
        let snap = Snapshot::from_json(text).expect("pre-1.1 snapshot parses");
        snap.check_schema_version().expect("same major is accepted");
        let node = &snap.nodes[0];
        assert_eq!(node.end_line, None);
        assert_eq!(node.signature, None);
        assert_eq!(node.decisions, None);
        let graph = snap.to_graph();
        let loaded = graph.nodes().find(|n| n.id == "pkg::m::f").expect("node rebuilt");
        assert_eq!(loaded.decisions, None, "absence survives the round trip");
    }

    /// Measured fields serialize when present and are omitted entirely
    /// when absent, so an unmeasured graph's snapshot bytes are exactly
    /// what earlier versions wrote.
    #[test]
    fn measured_fields_round_trip_and_unmeasured_stays_absent() {
        let text = r#"{
          "schema_version": "1.1",
          "nodes": [
            {"id": "pkg::m::f", "kind": "function", "label": "f",
             "loc": {"file": "src/m.rs", "line": 3}, "tier": "optional",
             "defines": [], "refs": [],
             "end_line": 9, "signature": "pub fn f(x: u32) -> u32", "decisions": 4},
            {"id": "pkg::m::g", "kind": "function", "label": "g",
             "loc": {"file": "src/m.rs", "line": 12}, "tier": "optional",
             "defines": [], "refs": []}
          ],
          "edges": [],
          "audit_head": "",
          "audit_log": []
        }"#;
        let snap = Snapshot::from_json(text).expect("1.1 snapshot parses");
        let f = &snap.nodes[0];
        assert_eq!(f.end_line, Some(9));
        assert_eq!(f.signature.as_deref(), Some("pub fn f(x: u32) -> u32"));
        assert_eq!(f.decisions, Some(4));
        let out = snap.to_json_pretty().expect("serializes");
        assert_eq!(
            out.matches("\"end_line\"").count(),
            1,
            "only the measured node carries the field: {out}"
        );
        let again = Snapshot::from_json(&out).expect("re-parses");
        assert_eq!(again.nodes[0].decisions, Some(4));
        assert_eq!(again.nodes[1].decisions, None);
    }

    /// Diagnostics recorded at index time travel with the snapshot:
    /// export → JSON → parse returns them intact, in builder order.
    #[test]
    fn diagnostics_round_trip_with_the_snapshot() {
        let diags = vec![
            Diagnostic::MultiDefinition {
                symbol: "pkg::m::f".to_string(),
                locations: vec![],
            },
            Diagnostic::UnresolvedCall {
                from: "pkg::m::g".to_string(),
                target: "missing".to_string(),
                line: 7,
            },
        ];
        let graph = Graph::from_parts(HashMap::new(), Vec::new(), HashMap::new(), HashMap::new());
        let snap = Snapshot::from_graph_with_diagnostics(&graph, &[], &diags);
        let out = snap.to_json_pretty().expect("serializes");
        assert!(
            out.contains("\"diagnostics\""),
            "recorded diagnostics are carried: {out}"
        );
        let again = Snapshot::from_json(&out).expect("re-parses");
        assert_eq!(again.diagnostics, diags, "diagnostics survive the round trip");
        assert_eq!(again, snap, "the whole snapshot is stable");
    }

    /// A graph with no recorded diagnostics writes exactly the bytes
    /// earlier versions wrote (no `diagnostics` key), and a snapshot
    /// written before the field existed parses with none recorded.
    #[test]
    fn absent_diagnostics_stay_absent() {
        let graph = Graph::from_parts(HashMap::new(), Vec::new(), HashMap::new(), HashMap::new());
        let snap = Snapshot::from_graph(&graph, &[]);
        let out = snap.to_json_pretty().expect("serializes");
        assert!(
            !out.contains("diagnostics"),
            "an unmeasured export gains no new bytes: {out}"
        );
        let legacy = Snapshot::from_json(&out).expect("own output re-parses");
        assert!(legacy.diagnostics.is_empty());
    }
}
