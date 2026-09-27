// Copyright (C) 2026 Nrupal Akolkar
// SPDX-License-Identifier: AGPL-3.0-or-later
// Part of codetopo — Owned by Nrupal Akolkar · Built with Muse by Meta.

//! Diagnostics — well-formedness violations reported as data, not panics
//! (CODE_SCHEMA §5: violations are first-class diagnostics, never silent).

use serde::{Deserialize, Serialize};
use std::fmt;

use crate::schema::{EdgeKind, Loc};

/// A well-formedness violation found while building the graph.
///
/// All variants serialize with serde so diagnostics can travel with snapshots
/// and query results.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Diagnostic {
    /// A non-external symbol defined in more than one place (CODE_SCHEMA §5.1).
    /// The builder keeps the first occurrence and ignores later ones;
    /// `locations[0]` is the kept definition.
    MultiDefinition {
        symbol: String,
        locations: Vec<Loc>,
    },
    /// A dependency cycle over `calls`/`imports` edges (CODE_SCHEMA §5.2).
    /// One diagnostic per strongly-connected component (size > 1, or a
    /// self-loop). `edge_kind` is `Calls` when any edge inside the SCC is a
    /// calls edge, otherwise `Imports`.
    Cycle {
        edge_kind: EdgeKind,
        cycle: Vec<String>,
    },
    /// A call target that resolved to no known symbol (CODE_SCHEMA §5.3).
    /// The edge is still created — to an `external::{target}` stub — so the
    /// call is never silently dropped. Emitted at most once per distinct
    /// target name per build.
    UnresolvedCall {
        from: String,
        target: String,
        line: u32,
    },
    /// An edge carried an attribute outside its kind's allow-list
    /// (CODE_SCHEMA §4). The offending attribute is dropped; the edge survives.
    /// `edge` is rendered as `from->to`.
    InvalidAttribute { edge: String, attr: String },
    /// A reference to a symbol (or module) that does not exist, e.g. an
    /// import whose `from_module_id` is unknown. The referencing edge is
    /// skipped.
    UnknownSymbol { id: String },
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Diagnostic::MultiDefinition { symbol, locations } => {
                let locs: Vec<String> = locations.iter().map(|l| l.to_string()).collect();
                write!(f, "multi-definition of '{}': {}", symbol, locs.join(", "))
            }
            Diagnostic::Cycle { edge_kind, cycle } => {
                // Render as a closed walk: a -> b -> a.
                let mut seq = cycle.clone();
                if let Some(first) = seq.first().cloned() {
                    seq.push(first);
                }
                write!(f, "cycle in {} edges: {}", edge_kind, seq.join(" -> "))
            }
            Diagnostic::UnresolvedCall { from, target, line } => {
                write!(f, "unresolved call from '{}' to '{}' at line {}", from, target, line)
            }
            Diagnostic::InvalidAttribute { edge, attr } => {
                write!(f, "invalid attribute '{}' on edge {}", attr, edge)
            }
            Diagnostic::UnknownSymbol { id } => {
                write!(f, "unknown symbol '{}'", id)
            }
        }
    }
}
