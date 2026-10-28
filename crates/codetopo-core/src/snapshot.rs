// Copyright (C) 2026 2351485 ALBERTA INC.
// SPDX-License-Identifier: AGPL-3.0-or-later
// Part of codetopo — Owned by 2351485 ALBERTA INC. (https://nrupalakolkar.com) · Built with Muse by Meta (https://muse.ai/join) — join Muse, then redeem code 101EEK in Settings within 48 hours of joining and we both earn tokens.

//! JSON snapshot pathway (CODE_SCHEMA §7: JSON is the universal export
//! pathway). A snapshot is the frozen, portable form of a [`Graph`]: nodes,
//! edges, the schema version the producer emitted, and the head of the
//! tamper-evident audit log it was exported under.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::graph::Graph;
use crate::schema::{Edge, EdgeKind, Node};

/// Schema version this implementation emits. Consumers reject major
/// mismatches loudly (CODE_SCHEMA §8); the 1.1 line adds the optional
/// measured fields of CODE_SCHEMA §3 (end line, signature, decision
/// counts) — additive, so 1.0 snapshots still read, their nodes simply
/// carry no measurements.
pub const SCHEMA_VERSION: &str = "1.1";

/// Portable frozen graph.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Snapshot {
    pub schema_version: String,
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
}

/// One entry of the tamper-evident audit log, as carried by a snapshot.
/// Field-for-field identical to the store crate's `AuditEntry`; defined here
/// so snapshots can transport the chain without core depending on the store
/// crate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
        let mut edges: Vec<Edge> = g.edges().cloned().collect();
        edges.sort_by(|a, b| {
            a.from
                .cmp(&b.from)
                .then_with(|| a.to.cmp(&b.to))
                .then_with(|| a.kind.as_str().cmp(b.kind.as_str()))
        });
        Snapshot {
            schema_version: SCHEMA_VERSION.to_string(),
            nodes: g.nodes().cloned().collect(),
            edges,
            audit_head: audit_log.last().map(|e| e.hash.clone()).unwrap_or_default(),
            audit_log: audit_log.to_vec(),
        }
    }

    /// The major component of a `MAJOR.MINOR...` schema version string
    /// (`"1.0-draft"` → `Some(1)`). `None` when the string does not start
    /// with a number — an unparseable version is never silently accepted.
    pub fn schema_major(version: &str) -> Option<u32> {
        version.split('.').next()?.parse().ok()
    }

    /// Enforce CODE_SCHEMA §8: a consumer rejects a snapshot whose schema
    /// major version differs from its own — loudly, never by guessing.
    /// Same-major minor drift is tolerated, per the §8 rule.
    pub fn check_schema_version(&self) -> Result<(), String> {
        let want = Self::schema_major(SCHEMA_VERSION).expect("own schema version parses");
        match Self::schema_major(&self.schema_version) {
            Some(m) if m == want => Ok(()),
            _ => Err(format!(
                "unsupported snapshot schema_version {:?} (this build supports major {want})",
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
        Graph::from_parts(nodes, self.edges.clone(), children, parents)
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
}
