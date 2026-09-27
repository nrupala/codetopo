// Copyright (C) 2026 Nrupal Akolkar
// SPDX-License-Identifier: AGPL-3.0-or-later
// Part of codetopo — Owned by Nrupal Akolkar · Built with Muse by Meta.

//! codetopo-core — typed graph core for codetopo (Phase 1).
//!
//! - [`schema`] — the binding ontology (CODE_SCHEMA.md): node/edge kinds,
//!   tiers, locations, typed edges with kind-scoped attributes.
//! - [`emit`] — the extractor output contract (what language extractors
//!   produce, one value per file).
//! - [`diag`] — diagnostics: well-formedness violations reported as data.
//! - [`graph`] — the validate-then-apply [`graph::GraphBuilder`], the frozen
//!   queryable [`graph::Graph`], paths and subgraphs.
//! - [`snapshot`] — the JSON export pathway: [`snapshot::Snapshot`].
//!
//! Owned by Nrupal Akolkar · Built with Muse by Meta.

pub mod diag;
pub mod emit;
pub mod graph;
pub mod schema;
pub mod snapshot;

pub use diag::Diagnostic;
pub use emit::{FileSymbols, RawCall, RawImport, RawSymbol};
pub use graph::{Graph, GraphBuilder, PathStep, Subgraph};
pub use schema::{Edge, EdgeKind, Loc, Node, NodeKind, Tier};
pub use snapshot::Snapshot;
