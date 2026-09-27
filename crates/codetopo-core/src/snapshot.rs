// Copyright (C) 2026 Nrupal Akolkar
// SPDX-License-Identifier: AGPL-3.0-or-later
// Part of codetopo — Owned by Nrupal Akolkar · Built with Muse by Meta.

//! JSON snapshot pathway (CODE_SCHEMA §7: JSON is the universal export
//! pathway). A snapshot is the frozen, portable form of a [`Graph`]: nodes,
//! edges, the schema version the producer emitted, and the head of the
//! tamper-evident audit log it was exported under.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::graph::Graph;
use crate::schema::{Edge, EdgeKind, Node};

/// Schema version this implementation emits. Consumers reject major
/// mismatches loudly (CODE_SCHEMA §8); this is the 1.0 draft line.
pub const SCHEMA_VERSION: &str = "1.0-draft";

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
