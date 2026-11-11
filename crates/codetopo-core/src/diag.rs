// Copyright (C) 2026 2351485 ALBERTA INC.
// SPDX-License-Identifier: AGPL-3.0-or-later
// Part of codetopo — Owned by 2351485 ALBERTA INC. (https://nrupalakolkar.com) · Built with Muse by Meta (https://muse.ai/join).

//! Diagnostics — well-formedness violations reported as data, not panics
//! (CODE_SCHEMA §5: violations are first-class diagnostics, never silent).

use serde::{Deserialize, Serialize};
use std::fmt;

use crate::schema::{EdgeKind, Loc};

/// A well-formedness violation found while building the graph.
///
/// All variants serialize with serde so diagnostics can travel with snapshots
/// and query results.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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
    /// An edge whose provenance class requires provenance attributes
    /// was offered without one (Domain-Profile Founding Spec §4, R2).
    /// The edge is refused — it does not enter the graph — and this
    /// diagnostic is the record of the refusal. `edge` is rendered as
    /// `from->to`; `missing` names the absent attributes.
    MissingProvenance {
        edge: String,
        kind: EdgeKind,
        missing: Vec<String>,
    },
    /// A tenant profile's declared articulation identity, evaluated by
    /// an extractor over the emitted graph, does not hold (Domain-
    /// Profile Founding Spec §1 well-formedness; format gap F-2, closed
    /// in Stage 5 of the domain-profile build). The diagnostic carries
    /// BOTH sides' measured values and the declared tolerance — the
    /// failure is reported, never repaired: the graph keeps the values
    /// the artefact actually computes. `identity` is the profile's
    /// identity name (e.g. `balance_sheet_equation`); `lhs`/`rhs` are
    /// the two sides' measured totals in the profile's currency units.
    ArticulationIdentity {
        identity: String,
        lhs: f64,
        rhs: f64,
        tolerance: f64,
    },
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
            Diagnostic::MissingProvenance { edge, kind, missing } => {
                write!(
                    f,
                    "missing provenance on {} edge {}: {}",
                    kind,
                    edge,
                    missing.join(", ")
                )
            }
            Diagnostic::ArticulationIdentity { identity, lhs, rhs, tolerance } => {
                write!(
                    f,
                    "articulation identity '{}' does not hold: lhs {} vs rhs {} (difference {}, tolerance {})",
                    identity,
                    lhs,
                    rhs,
                    (lhs - rhs).abs(),
                    tolerance
                )
            }
        }
    }
}
