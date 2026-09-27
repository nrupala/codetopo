// Copyright (C) 2026 Nrupal Akolkar
// SPDX-License-Identifier: AGPL-3.0-or-later
// Part of codetopo — Owned by Nrupal Akolkar · Built with Muse by Meta.

//! Typed code-entity ontology (CODE_SCHEMA.md §2–§4, spec version 1.0-draft).
//!
//! This module is the Rust rendering of the binding ontology: every extractor
//! emits these types, every query reads them, every renderer consumes them.
//! Wire format is snake_case JSON per the schema's examples
//! (`{"kind": "method", ...}`).

use serde::{Deserialize, Serialize};
use std::fmt;

/// Node kinds — CODE_SCHEMA §2.
///
/// Serialized snake_case so the JSON matches the spec exactly.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum NodeKind {
    Package,
    Module,
    File,
    Class,
    Interface,
    Function,
    Method,
    Variable,
    External,
    Doc,
}

/// Edge kinds — CODE_SCHEMA §4.
///
/// Serialized snake_case. `Ord` is derived so adjacency lists and query
/// outputs can be kept in a deterministic order.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum EdgeKind {
    Imports,
    Calls,
    Inherits,
    Implements,
    Defines,
    References,
    Contains,
    Reads,
    Writes,
}

impl EdgeKind {
    /// Attribute allow-list per CODE_SCHEMA §4. No edge may carry attributes
    /// outside its row; `GraphBuilder` enforces this and reports violations as
    /// [`crate::diag::Diagnostic::InvalidAttribute`].
    ///
    /// Note: `line` also exists as a first-class field on [`Edge`]; it is
    /// listed as allowed here so that carrying it inside `attrs` is not itself
    /// a violation.
    pub fn allowed_attrs(self) -> &'static [&'static str] {
        match self {
            EdgeKind::Imports => &["bindings", "line"],
            EdgeKind::Calls => &["line", "arg_shapes"],
            EdgeKind::Inherits => &["bases"],
            EdgeKind::Implements => &["members"],
            EdgeKind::Defines => &["line"],
            EdgeKind::References => &["line", "kind"],
            // `contains` is pure hierarchy — no attributes at all.
            EdgeKind::Contains => &[],
            EdgeKind::Reads | EdgeKind::Writes => &["line"],
        }
    }

    /// Deterministic rank used to order adjacency lists and query outputs.
    /// Follows declaration order (imports < calls < inherits < …).
    pub fn rank(self) -> u8 {
        match self {
            EdgeKind::Imports => 0,
            EdgeKind::Calls => 1,
            EdgeKind::Inherits => 2,
            EdgeKind::Implements => 3,
            EdgeKind::Defines => 4,
            EdgeKind::References => 5,
            EdgeKind::Contains => 6,
            EdgeKind::Reads => 7,
            EdgeKind::Writes => 8,
        }
    }

    /// The snake_case wire name (`"imports"`, `"calls"`, …).
    pub fn as_str(self) -> &'static str {
        match self {
            EdgeKind::Imports => "imports",
            EdgeKind::Calls => "calls",
            EdgeKind::Inherits => "inherits",
            EdgeKind::Implements => "implements",
            EdgeKind::Defines => "defines",
            EdgeKind::References => "references",
            EdgeKind::Contains => "contains",
            EdgeKind::Reads => "reads",
            EdgeKind::Writes => "writes",
        }
    }
}

impl fmt::Display for EdgeKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Queryable priority tier — CODE_SCHEMA §2.
///
/// The extractor contract ([`crate::emit::RawSymbol`]) carries no tier in this
/// version, so nodes built from extractor output default to `Optional`
/// ("not classified by the extractor") rather than inventing a priority.
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

/// Minimal node record — the write path (CODE_SCHEMA §3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Node {
    pub id: String,
    pub kind: NodeKind,
    pub label: String,
    pub loc: Loc,
    /// Queryable priority. Defaults to `Optional` on deserialization when the
    /// producer did not classify the node.
    #[serde(default)]
    pub tier: Tier,
    /// Symbols defined here (the marimo contract, verbatim from the extractor).
    #[serde(default)]
    pub defines: Vec<String>,
    /// Symbols referenced here (the marimo contract, verbatim from the extractor).
    #[serde(default)]
    pub refs: Vec<String>,
}

/// Typed edge — CODE_SCHEMA §4.
///
/// `line` is a first-class field; every other attribute lives in `attrs` and
/// must belong to the edge kind's allow-list
/// ([`EdgeKind::allowed_attrs`]).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Edge {
    pub from: String,
    pub to: String,
    pub kind: EdgeKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<u32>,
    #[serde(default)]
    pub attrs: serde_json::Map<String, serde_json::Value>,
}
