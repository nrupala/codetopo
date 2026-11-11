// Copyright (C) 2026 2351485 ALBERTA INC.
// SPDX-License-Identifier: AGPL-3.0-or-later
// Part of codetopo — Owned by 2351485 ALBERTA INC. (https://nrupalakolkar.com) · Built with Muse by Meta (https://muse.ai/join).

//! Graph record types (core schema 2.0).
//!
//! Pre-2.0 this module carried CODE_SCHEMA's closed dictionary as Rust
//! enums: `NodeKind` / `EdgeKind` variants *were* the code domain's
//! vocabulary, and the core matched on them exhaustively. Under the
//! domain-profile mechanism (see [`crate::profile`]) the dictionary is
//! profile data: a kind is a name owned by the loaded profile, carried
//! here as a string newtype. The CODE dictionary survives as
//! associated constants (`NodeKind::Function`, `EdgeKind::Calls`, …)
//! so construction and pattern sites read as before, but dictionary
//! *questions* — tiers, attribute allow-lists, edge sets, the boundary
//! test — are answered by the profile's tables, never by matching on
//! a domain's vocabulary in the core.
//!
//! Wire format is unchanged: kinds serialize as their bare snake_case
//! names, exactly as the schema's examples show.

use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};


/// A node kind: a profile-declared snake_case name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct NodeKind(&'static str);

// The CODE constants keep the dictionary's own names (Package,
// Calls, ...) so construction and pattern sites read exactly as they
// did when the kinds were enum variants -- the naming is the API.
#[allow(non_upper_case_globals)]
impl NodeKind {
    pub const Package: NodeKind = NodeKind("package");
    pub const Module: NodeKind = NodeKind("module");
    pub const File: NodeKind = NodeKind("file");
    pub const Class: NodeKind = NodeKind("class");
    pub const Interface: NodeKind = NodeKind("interface");
    pub const Function: NodeKind = NodeKind("function");
    pub const Method: NodeKind = NodeKind("method");
    pub const Variable: NodeKind = NodeKind("variable");
    pub const External: NodeKind = NodeKind("external");
    pub const Doc: NodeKind = NodeKind("doc");

    /// Construct from a wire name. Names longer than the intern table
    /// or unknown to it are stored via the owned fallback in
    /// [`NodeKind::new`]'s sibling type — see the note on `KindName`.
    pub fn new(name: &str) -> Self {
        NodeKind(intern(name))
    }

    pub fn as_str(&self) -> &'static str {
        self.0
    }
}

/// Intern a kind name. The kind vocabulary of any one profile is
/// small and fixed at load; interning through a leaked table keeps
/// kinds `Copy` and their values `'static`, which the whole engine's
/// signatures assume. Unknown names intern on first sight.
fn intern(name: &str) -> &'static str {
    use std::sync::OnceLock;
    static TABLE: OnceLock<std::sync::Mutex<std::collections::HashSet<&'static str>>> =
        OnceLock::new();
    let table = TABLE.get_or_init(|| std::sync::Mutex::new(std::collections::HashSet::new()));
    let mut guard = table.lock().expect("kind intern table");
    if let Some(&existing) = guard.get(name) {
        return existing;
    }
    let leaked: &'static str = Box::leak(name.to_string().into_boxed_str());
    guard.insert(leaked);
    leaked
}

impl fmt::Display for NodeKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}

impl Serialize for NodeKind {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(self.0)
    }
}

impl<'de> Deserialize<'de> for NodeKind {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let name = String::deserialize(d)?;
        Ok(NodeKind::new(&name))
    }
}

/// An edge kind: a profile-declared snake_case name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct EdgeKind(&'static str);

#[allow(non_upper_case_globals)]
impl EdgeKind {
    pub const Imports: EdgeKind = EdgeKind("imports");
    pub const Calls: EdgeKind = EdgeKind("calls");
    pub const Inherits: EdgeKind = EdgeKind("inherits");
    pub const Implements: EdgeKind = EdgeKind("implements");
    pub const Defines: EdgeKind = EdgeKind("defines");
    pub const References: EdgeKind = EdgeKind("references");
    pub const Contains: EdgeKind = EdgeKind("contains");
    pub const Reads: EdgeKind = EdgeKind("reads");
    pub const Writes: EdgeKind = EdgeKind("writes");

    pub fn new(name: &str) -> Self {
        EdgeKind(intern(name))
    }

    pub fn as_str(&self) -> &'static str {
        self.0
    }

    /// Deterministic rank used to order adjacency lists and query
    /// outputs where no profile is in hand: the CODE declaration
    /// order for CODE kinds (identical to the pre-2.0 enum rank, so
    /// restored-graph ordering cannot drift), then name order for
    /// any other profile's kinds. Profile-aware ordering lives in
    /// [`crate::profile::DomainProfile::edge_rank`]; the two agree on
    /// the CODE profile.
    pub fn rank(self) -> (usize, &'static str) {
        let base = match self.0 {
            "imports" => 0,
            "calls" => 1,
            "inherits" => 2,
            "implements" => 3,
            "defines" => 4,
            "references" => 5,
            "contains" => 6,
            "reads" => 7,
            "writes" => 8,
            _ => usize::MAX,
        };
        (base, self.0)
    }
}

impl fmt::Display for EdgeKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}

impl Serialize for EdgeKind {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(self.0)
    }
}

impl<'de> Deserialize<'de> for EdgeKind {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let name = String::deserialize(d)?;
        Ok(EdgeKind::new(&name))
    }
}

/// Queryable priority tier.
///
/// The tier *vocabulary* (core / recommended / optional) is core law —
/// §3's three classes — while the kind→tier mapping is profile data
/// ([`crate::profile::DomainProfile::tier_for`]).
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, Default,
)]
#[serde(rename_all = "snake_case")]
pub enum Tier {
    Core,
    Recommended,
    #[default]
    Optional,
}

/// Source location: file + 1-based line, optional column.
///
/// `column` is omitted from JSON when absent.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Loc {
    pub file: String,
    pub line: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub column: Option<u32>,
}

impl fmt::Display for Loc {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.column {
            Some(c) => write!(f, "{}:{}:{}", self.file, self.line, c),
            None => write!(f, "{}:{}", self.file, self.line),
        }
    }
}

/// Minimal node record — the write path (§3 record shape; the code
/// vocabulary that fills it is the profile's).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Node {
    pub id: String,
    pub kind: NodeKind,
    pub label: String,
    pub loc: Loc,
    /// Queryable priority. Defaults to `Optional` on deserialization
    /// when the producer did not classify the node; the builder
    /// classifies by the loaded profile's tier rule.
    #[serde(default)]
    pub tier: Tier,
    /// Symbols defined here (the marimo contract, verbatim from the extractor).
    #[serde(default)]
    pub defines: Vec<String>,
    /// Symbols referenced here (the marimo contract, verbatim from the extractor).
    #[serde(default)]
    pub refs: Vec<String>,
    /// Last source line of the declaration (1-based, inclusive) —
    /// a core-reserved measured field (§3, schema 1.1). An
    /// extractor-emitted measurement: `None` is *not measured*,
    /// never a span of zero. Snapshots written before schema 1.1
    /// carry no value here and read as absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_line: Option<u32>,
    /// The declaration text — body and initializer excluded,
    /// whitespace collapsed (§3). A core-reserved measured field.
    /// `None` when not measured.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature: Option<String>,
    /// Decision points in the symbol's body, counted per the loaded
    /// profile's `decisions` extraction rule (for CODE: §3.2's
    /// per-language enumeration, carried in the profile itself). A
    /// core-reserved measured field. `None` is *not measured*,
    /// never zero.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decisions: Option<u32>,
    /// Profile-declared measured fields beyond the core-reserved
    /// three, keyed by the profile's field name (founding spec §1.2
    /// part 6, the hybrid of governing default 3). Absence of a key
    /// is *not measured* for that field, never a default. Omitted
    /// from the wire entirely when empty, so a CODE graph's records
    /// keep their exact 1.x shape.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub measures: BTreeMap<String, serde_json::Value>,
}

impl Node {
    /// Read a measured field by name: the core-reserved fields answer
    /// from their first-class slots; any other name answers from the
    /// profile `measures` map. `None` is *not measured*.
    pub fn measure(&self, name: &str) -> Option<serde_json::Value> {
        match name {
            "end_line" => self.end_line.map(serde_json::Value::from),
            "signature" => self.signature.clone().map(serde_json::Value::from),
            "decisions" => self.decisions.map(serde_json::Value::from),
            other => self.measures.get(other).cloned(),
        }
    }
}

/// Typed edge (§4 record shape).
///
/// `line` is a first-class field; every other attribute lives in
/// `attrs` and must belong to the edge kind's allow-list in the
/// loaded profile ([`crate::profile::DomainProfile::allowed_attrs`]).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Edge {
    pub from: String,
    pub to: String,
    pub kind: EdgeKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<u32>,
    #[serde(default)]
    pub attrs: serde_json::Map<String, serde_json::Value>,
}
