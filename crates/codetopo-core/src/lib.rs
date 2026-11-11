// Copyright (C) 2026 2351485 ALBERTA INC.
// SPDX-License-Identifier: AGPL-3.0-or-later
// Part of codetopo — Owned by 2351485 ALBERTA INC. (https://nrupalakolkar.com) · Built with Muse by Meta (https://muse.ai/join).

//! codetopo-core — typed graph core for codetopo (Phase 1).
//!
//! - [`schema`] — node/edge records, tiers, locations, typed edges.
//! - [`profile`] — the domain profile: the dictionary (kinds, edge
//!   sets, provenance classes, sibling measures) as data, loaded and
//!   validated; the CODE profile is `profiles/code.profile.json`,
//!   the extraction of CODE_SCHEMA v1.2 (core schema 2.0).
//! - [`emit`] — the extractor output contract (what language extractors
//!   produce, one value per file).
//! - [`diag`] — diagnostics: well-formedness violations reported as data.
//! - [`graph`] — the validate-then-apply [`graph::GraphBuilder`], the frozen
//!   queryable [`graph::Graph`], paths and subgraphs.
//! - [`measure`] — measurements over the frozen graph (the analytics-depth
//!   layer): the symbol dossier of DESIGN §3, rankings, graph diffs.
//! - [`snapshot`] — the JSON export pathway: [`snapshot::Snapshot`].
//!
//! Owned by 2351485 ALBERTA INC. (https://nrupalakolkar.com) · Built with Muse by Meta (https://muse.ai/join).

pub mod centrality;
pub mod communities;
pub mod coupling;
pub mod diag;
pub mod history;
pub mod emit;
pub mod force;
pub mod graph;
pub mod measure;
pub mod profile;
pub mod projection;
pub mod render;
pub mod schema;
pub mod snapshot;
pub mod topology;

pub use communities::{
    CommunityPartition, CommunitySummary, LeidenDetector, NodeCommunity, LEIDEN_SEED,
};

/// The published method of every coupling/cohesion metric, keyed by
/// the field name the surfaces report — since schema 2.0 this is a
/// view over the CODE profile's sibling registry (founding spec
/// §1.4: the registry, not a Rust constant, is their home). Kept as
/// a function because the registry is data now.
pub fn metric_methods() -> Vec<(String, String)> {
    DomainProfile::code().metric_methods()
}

/// The published method of every history measurement, from the CODE
/// profile's history parameterisation (founding spec D5).
pub fn history_methods() -> Vec<(String, String)> {
    DomainProfile::code()
        .history
        .as_ref()
        .map(|h| {
            h.methods
                .iter()
                .map(|(n, t)| (n.clone(), t.clone()))
                .collect()
        })
        .unwrap_or_default()
}

/// The communities method text (the profile-independent §9 method,
/// parameterised by the CODE profile's dependency set).
pub fn communities_method() -> String {
    DomainProfile::code()
        .core_method("communities")
        .expect("core method registered")
}

/// The topo-sort method text, likewise.
pub fn topo_sort_method() -> String {
    DomainProfile::code()
        .core_method("topo_sort")
        .expect("core method registered")
}
pub use coupling::{GraphMetrics, PackageMetrics, TypeMetrics};
pub use history::{graph_history, CoupledFiles, FileHotspot, HistoryInput, HistoryReport};
pub use diag::Diagnostic;
pub use emit::{FileSymbols, RawCall, RawHeritage, RawImport, RawSymbol};
pub use graph::{Graph, GraphBuilder, PathStep, Subgraph};
pub use measure::{
    diff_graphs, GraphDiff, ImpactEntry, NodeChange, RankAnswer, RankMetric, RankedSymbol,
    Ranking, SymbolDossier, SymbolMeasurements,
};
pub use projection::{AnalyticsProjection, CommunityDetector};
pub use force::{
    force_layout, render_svg_v2, ForceLayout, ForceNode, ForceParams, LayoutSpecV2,
};
pub use render::{layout, render_svg, LayoutEdge, LayoutNode, LayoutSpec};
pub use profile::{
    Admission, AggregateKindDef, AttrDef, BoundaryDef, ConformanceDef, ContainmentDef,
    CoreCompat, DomainProfile, EdgeKindDef, EdgeSets, HistoryDef, MeasureFieldDef,
    NodeKindDef, ProvenanceClass, SiblingDef, SiblingInputs, WellFormednessDef,
    PROFILE_FORMAT, CODE_PROFILE_JSON,
};
pub use topology::{TopoGroup, TopoOrder};
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
