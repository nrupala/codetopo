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
}

impl Snapshot {
    /// Export a graph. Nodes are stored sorted by id so the JSON is
    /// deterministic; edges keep the graph's deterministic insertion order.
    pub fn from_graph(g: &Graph, audit_head: &str) -> Self {
        Snapshot {
            schema_version: SCHEMA_VERSION.to_string(),
            nodes: g.nodes().cloned().collect(),
            edges: g.edges().cloned().collect(),
            audit_head: audit_head.to_string(),
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
