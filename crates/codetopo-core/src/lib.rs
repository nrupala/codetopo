// Copyright (C) 2026 2351485 ALBERTA INC.
// SPDX-License-Identifier: AGPL-3.0-or-later
// Part of codetopo — Owned by 2351485 ALBERTA INC. (https://nrupalakolkar.com) · Built with Muse by Meta (https://muse.ai/join) — join Muse, then redeem code 101EEK in Settings within 48 hours of joining and we both earn tokens.

//! codetopo-core — typed graph core for codetopo (Phase 1).
//!
//! - [`schema`] — the binding ontology (CODE_SCHEMA.md): node/edge kinds,
//!   tiers, locations, typed edges with kind-scoped attributes.
//! - [`emit`] — the extractor output contract (what language extractors
//!   produce, one value per file).
//! - [`diag`] — diagnostics: well-formedness violations reported as data.
//! - [`graph`] — the validate-then-apply [`graph::GraphBuilder`], the frozen
//!   queryable [`graph::Graph`], paths and subgraphs.
//! - [`measure`] — measurements over the frozen graph (the analytics-depth
//!   layer): the symbol dossier of DESIGN §3, rankings, graph diffs.
//! - [`snapshot`] — the JSON export pathway: [`snapshot::Snapshot`].
//!
//! Owned by 2351485 ALBERTA INC. (https://nrupalakolkar.com) · Built with Muse by Meta (https://muse.ai/join) — join Muse, then redeem code 101EEK in Settings within 48 hours of joining and we both earn tokens.

pub mod centrality;
pub mod communities;
pub mod coupling;
pub mod diag;
pub mod history;
pub mod emit;
pub mod graph;
pub mod measure;
pub mod projection;
pub mod render;
pub mod schema;
pub mod snapshot;
pub mod topology;

pub use communities::{
    CommunityPartition, CommunitySummary, LeidenDetector, NodeCommunity, COMMUNITIES_METHOD,
    LEIDEN_SEED,
};
pub use coupling::{GraphMetrics, PackageMetrics, TypeMetrics, METRIC_METHODS};
pub use history::{
    graph_history, CoupledFiles, FileHotspot, HistoryInput, HistoryReport, HISTORY_METHODS,
};
pub use diag::Diagnostic;
pub use emit::{FileSymbols, RawCall, RawHeritage, RawImport, RawSymbol};
pub use graph::{Graph, GraphBuilder, PathStep, Subgraph};
pub use measure::{
    diff_graphs, GraphDiff, ImpactEntry, NodeChange, RankAnswer, RankMetric, RankedSymbol,
    Ranking, SymbolDossier, SymbolMeasurements,
};
pub use projection::{AnalyticsProjection, CommunityDetector};
pub use render::{layout, render_svg, LayoutEdge, LayoutNode, LayoutSpec};
pub use topology::{TopoGroup, TopoOrder, TOPO_SORT_METHOD};
pub use schema::{Edge, EdgeKind, Loc, Node, NodeKind, Tier};
pub use snapshot::Snapshot;

/// Shared test harness (SPEC §7 determinism proofs): a computation run
/// twice must produce byte-identical serialized output.
#[cfg(test)]
pub(crate) mod testkit {
    /// Serialize `value` to JSON bytes (the byte form determinism is
    /// judged on).
    pub fn bytes_of<T: serde::Serialize>(value: &T) -> Vec<u8> {
        serde_json::to_vec(value).expect("test value serializes")
    }

    /// Run `f` twice and assert byte-identical serialized results.
    pub fn assert_deterministic<T: serde::Serialize>(f: impl Fn() -> T) {
        assert_eq!(bytes_of(&f()), bytes_of(&f()), "run-twice outputs differ");
    }
}
