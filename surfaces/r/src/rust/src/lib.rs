// Copyright (C) 2026 2351485 ALBERTA INC.
// SPDX-License-Identifier: AGPL-3.0-or-later
// Part of codetopo — Owned by 2351485 ALBERTA INC. (https://nrupalakolkar.com) · Built with Muse by Meta (https://muse.ai/join).

//! codetopo (R surface) — an extendr transport over [`codetopo_cli`]'s
//! library, exactly as the diaspora boundary doctrine prescribes: the
//! boundary exchanges CODE_SCHEMA JSON. Indexing calls
//! `codetopo_cli::index_repo`; every query returns the same
//! `serde_json::Value` the CLI library builds for the CLI / HTTP / MCP
//! surfaces, so the R answers are those answers by construction —
//! parity is inherited, not re-implemented. Where the CLI library only
//! ships a print form (stats, the id-list closures, search, dossier,
//! path, neighborhood, snapshot, verify), this crate rebuilds that
//! print form's JSON shape field-for-field from the same core calls
//! (the identical approach the JavaScript napi surface takes).
//!
//! All values cross the boundary as JSON text; the R layer parses them
//! with `jsonlite::fromJSON(..., simplifyVector = FALSE)` so absent
//! measurements stay absent, never coerced to 0.
//!
//! Stateless by design: every function takes the handle's coordinates
//! (`db_path` for a database-born handle, `snapshot_json` for a
//! snapshot-born one — exactly one set) and loads the frozen graph per
//! call. The R handle is an environment holding those coordinates; no
//! Rust state lives across calls, so there are no external-pointer
//! lifetime hazards.
//!
//! Errors cross the boundary on the success channel as a JSON envelope
//! `{"__codetopo_error__": {"code", "message"}}`, which the R layer
//! detects and re-raises as an R error carrying the core's code. This
//! keeps the code/message pair byte-exact regardless of how the
//! extendr version at hand formats propagated Rust errors. Codes use
//! the existing vocabulary (`unknown_symbol`,
//! `unsupported_schema_version`, `invalid_param`, `missing_param`,
//! `unknown_index`, `io_error`, `store_error`, …).

use std::path::{Path, PathBuf};

use codetopo_cli::CliError;
use codetopo_core::snapshot::SnapshotAuditEntry;
use codetopo_core::{Graph, RankMetric, Snapshot};
use codetopo_store::{AuditLog, Store};
use extendr_api::prelude::*;
use serde_json::{json, Value};

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

type CodedResult<T> = std::result::Result<T, (String, String)>;

fn coded<T>(code: &str, message: impl Into<String>) -> CodedResult<T> {
    Err((code.to_string(), message.into()))
}

fn cli_error(err: CliError) -> (String, String) {
    match &err {
        CliError::UnknownId(id) => (
            "unknown_symbol".to_string(),
            format!("unknown symbol '{id}': not present in the graph"),
        ),
        CliError::Usage(msg) if msg.contains("unsupported") && msg.contains("schema") => {
            ("unsupported_schema_version".to_string(), msg.clone())
        }
        CliError::Usage(msg) => ("invalid_param".to_string(), msg.clone()),
        CliError::Store(e) => ("store_error".to_string(), format!("store error: {e}")),
        CliError::Io(e) => ("io_error".to_string(), format!("io error: {e}")),
        CliError::Json(e) => ("invalid_json".to_string(), format!("json error: {e}")),
    }
}

fn unknown_symbol(id: &str) -> (String, String) {
    (
        "unknown_symbol".to_string(),
        format!("unknown symbol '{id}': not present in the graph"),
    )
}

/// Run a fallible body and encode its outcome as the boundary string:
/// the answer JSON on success, the error envelope on failure.
fn boundary(body: impl FnOnce() -> CodedResult<String>) -> String {
    match body() {
        Ok(text) => text,
        Err((code, message)) => {
            json!({ "__codetopo_error__": { "code": code, "message": message } }).to_string()
        }
    }
}

fn to_json_string(value: &Value) -> CodedResult<String> {
    serde_json::to_string(value)
        .map_err(|e| ("internal".to_string(), format!("answer does not serialize: {e}")))
}

fn parse_snapshot(snapshot_json: &str) -> CodedResult<Snapshot> {
    let snap = Snapshot::from_json(snapshot_json).map_err(|e| {
        (
            "invalid_json".to_string(),
            format!("snapshot does not parse: {e}"),
        )
    })?;
    snap.check_schema_version()
        .map_err(|msg| ("unsupported_schema_version".to_string(), msg))?;
    Ok(snap)
}

// ---------------------------------------------------------------------------
// Handle loading
// ---------------------------------------------------------------------------

/// A loaded graph plus the context its queries need: the database it
/// came from (diagnostics and the audit chain live in the store, not
/// the graph) and the audit entries a snapshot of it must carry.
/// Snapshot-born handles have no database; their audit entries are the
/// snapshot's own. (Field-for-field the napi surface's GraphHandle.)
struct Loaded {
    graph: Graph,
    db_path: Option<String>,
    audit_entries: Vec<SnapshotAuditEntry>,
}

impl Loaded {
    fn require(&self, id: &str) -> CodedResult<()> {
        if self.graph.get(id).is_some() {
            Ok(())
        } else {
            Err(unknown_symbol(id))
        }
    }

    fn audit_entries_fresh(&self) -> Vec<SnapshotAuditEntry> {
        // A database handle re-reads the chain so a snapshot taken
        // after a re-index binds the current head; a snapshot-born
        // handle replays its own.
        if let Some(db) = &self.db_path {
            if let Ok(store) = Store::open(Path::new(db)) {
                if let Ok(audit) = store.load_audit() {
                    return audit.snapshot_entries();
                }
            }
        }
        self.audit_entries.clone()
    }
}

fn opt(s: &str) -> Option<String> {
    if s.is_empty() {
        None
    } else {
        Some(s.to_string())
    }
}

/// Load the handle identified by (`db_path`, `snapshot_json`); exactly
/// one must be non-empty.
fn load(db_path: &str, snapshot_json: &str) -> CodedResult<Loaded> {
    match (opt(db_path), opt(snapshot_json)) {
        (Some(db), None) => {
            let path = Path::new(&db);
            if !path.is_file() {
                return coded(
                    "unknown_index",
                    format!("unknown index: no database file at '{db}'"),
                );
            }
            let graph = codetopo_cli::load_graph_from_db(path).map_err(cli_error)?;
            let audit_entries = Store::open(path)
                .and_then(|store| store.load_audit())
                .map(|audit| audit.snapshot_entries())
                .unwrap_or_default();
            Ok(Loaded {
                graph,
                db_path: Some(db),
                audit_entries,
            })
        }
        (None, Some(text)) => {
            let snap = parse_snapshot(&text)?;
            let audit_entries = snap.audit_log.clone();
            Ok(Loaded {
                graph: snap.to_graph(),
                db_path: None,
                audit_entries,
            })
        }
        _ => coded(
            "invalid_param",
            "a handle must carry exactly one of db_path / snapshot_json",
        ),
    }
}

fn closure_json(loaded: &Loaded, relation: &str, id: &str, limit: i32, expand: bool) -> CodedResult<String> {
    loaded.require(id)?;
    let limit = limit.max(0) as usize;
    let members: Vec<String> = match relation {
        "descendants" => loaded.graph.descendants(id),
        "ancestors" => loaded.graph.ancestors(id),
        _ => loaded.graph.blast_radius(id),
    };
    let total = members.len();
    let shown: &[String] = &members[..total.min(limit)];
    // Field-for-field the CLI library's JSON id-list form; with
    // `expand`, the shown members are the full node objects, as on the
    // MCP surface.
    let nodes: Vec<Value> = if expand {
        shown
            .iter()
            .filter_map(|mid| loaded.graph.get(mid))
            .map(|n| serde_json::to_value(n).expect("a node always serializes"))
            .collect()
    } else {
        shown.iter().map(|mid| json!(mid)).collect()
    };
    to_json_string(&json!({
        "relation": relation,
        "node": id,
        "total": total,
        "expand": expand,
        "limit": limit,
        "truncated": total > shown.len(),
        "nodes": nodes,
    }))
}

// ---------------------------------------------------------------------------
// Provenance + schema
// ---------------------------------------------------------------------------

/// The version of the bound `codetopo-core` crate, captured by
/// build.rs from that crate's own manifest.
#[extendr]
fn ct_core_version() -> String {
    env!("CODETOPO_CORE_VERSION").to_string()
}

/// The CODE_SCHEMA version the bound core emits — the core's own
/// constant, never a restated literal.
#[extendr]
fn ct_schema_version() -> String {
    codetopo_core::snapshot::SCHEMA_VERSION.to_string()
}

/// `codetopo_describe_schema`: the schema description every surface serves.
#[extendr]
fn ct_describe_schema() -> String {
    boundary(|| to_json_string(&codetopo_cli::describe_schema()))
}

// ---------------------------------------------------------------------------
// Index / restore
// ---------------------------------------------------------------------------

/// `codetopo_index` — walk, extract, build, persist. Returns the
/// index report JSON.
#[extendr]
fn ct_index(repo_path: &str, db_path: &str, package: &str) -> String {
    boundary(|| {
        let repo = PathBuf::from(repo_path);
        let package = if package.is_empty() {
            codetopo_cli::default_package_name(&repo)
        } else {
            package.to_string()
        };
        let report =
            codetopo_cli::index_repo(&repo, Path::new(db_path), &package, None, None)
                .map_err(cli_error)?;
        to_json_string(&json!({
            "package": package,
            "repo_path": repo_path,
            "db_path": db_path,
            "files_indexed": report.files_indexed,
            "files_failed": report.files_failed,
            "nodes": report.nodes,
            "edges": report.edges,
            "diagnostics": report.diagnostics,
        }))
    })
}

/// Restore snapshot JSON into a database at `db_path` — the full
/// verification gate, as on the CLI: schema major, audit chain
/// recomputing to the declared head. A snapshot with no audit trail is
/// refused (there is nothing to verify it against). Returns a JSON
/// report `{nodes, edges, verified}`.
#[extendr]
fn ct_restore(snapshot_json: &str, db_path: &str) -> String {
    boundary(|| {
        let snap = parse_snapshot(snapshot_json)?;
        if snap.audit_log.is_empty() {
            return coded(
                "unverified_snapshot",
                "snapshot carries no audit trail — nothing to verify it against",
            );
        }
        let audit = AuditLog::from_snapshot_entries(&snap.audit_log);
        if !audit.verify() || audit.head() != snap.audit_head {
            return coded(
                "audit_chain_invalid",
                "snapshot audit chain does not verify against its declared head",
            );
        }
        let graph = snap.to_graph();
        let nodes = graph.node_count();
        let edges = graph.edge_count();
        let mut store = Store::open(Path::new(db_path))
            .map_err(|e| ("store_error".to_string(), format!("store error: {e}")))?;
        store
            .replace_graph(&graph)
            .map_err(|e| ("store_error".to_string(), format!("store error: {e}")))?;
        store
            .save_audit(&audit)
            .map_err(|e| ("store_error".to_string(), format!("store error: {e}")))?;
        to_json_string(&json!({ "nodes": nodes, "edges": edges, "verified": true }))
    })
}

// ---------------------------------------------------------------------------
// Queries (each takes the handle coordinates first)
// ---------------------------------------------------------------------------

/// `codetopo_stats` — the CLI library's JSON stats shape.
#[extendr]
fn ct_stats(db_path: &str, snapshot_json: &str) -> String {
    boundary(|| {
        let loaded = load(db_path, snapshot_json)?;
        let mut node_kinds = serde_json::Map::new();
        for kind in codetopo_cli::all_node_kinds() {
            let count = loaded.graph.nodes().filter(|n| n.kind == kind).count();
            if count > 0 {
                node_kinds.insert(codetopo_cli::node_kind_name(kind).to_string(), count.into());
            }
        }
        let mut edge_kinds = serde_json::Map::new();
        for kind in codetopo_cli::all_edge_kinds() {
            let count = loaded.graph.edge_count_by_kind(kind);
            if count > 0 {
                edge_kinds.insert(kind.as_str().to_string(), count.into());
            }
        }
        to_json_string(&json!({
            "nodes": { "total": loaded.graph.node_count(), "by_kind": node_kinds },
            "edges": { "total": loaded.graph.edge_count(), "by_kind": edge_kinds },
        }))
    })
}

/// `codetopo_search_symbols` — `{query, total, results}`.
#[extendr]
fn ct_search_symbols(db_path: &str, snapshot_json: &str, query: &str, limit: i32) -> String {
    boundary(|| {
        let loaded = load(db_path, snapshot_json)?;
        let hits = loaded.graph.search_symbols(query, limit.max(0) as usize);
        to_json_string(&json!({ "query": query, "total": hits.len(), "results": hits }))
    })
}

/// `codetopo_get_symbol` — the symbol dossier, serialized by the core
/// type itself.
#[extendr]
fn ct_get_symbol(db_path: &str, snapshot_json: &str, id: &str) -> String {
    boundary(|| {
        let loaded = load(db_path, snapshot_json)?;
        let dossier = loaded
            .graph
            .dossier(id)
            .ok_or_else(|| unknown_symbol(id))?;
        to_json_string(
            &serde_json::to_value(dossier).map_err(|e| {
                (
                    "internal".to_string(),
                    format!("dossier does not serialize: {e}"),
                )
            })?,
        )
    })
}

/// `codetopo_descendants` / `codetopo_ancestors` /
/// `codetopo_blast_radius` — the transitive closures (`relation`
/// selects which).
#[extendr]
fn ct_closure(
    db_path: &str,
    snapshot_json: &str,
    relation: &str,
    id: &str,
    limit: i32,
    expand: bool,
) -> String {
    boundary(|| {
        let loaded = load(db_path, snapshot_json)?;
        closure_json(&loaded, relation, id, limit, expand)
    })
}

/// `codetopo_path` — the shortest directed typed path. JSON `null`
/// when both endpoints exist but no directed path joins them; an
/// unknown endpoint is an `unknown_symbol` error.
#[extendr]
fn ct_path(db_path: &str, snapshot_json: &str, from: &str, to: &str) -> String {
    boundary(|| {
        let loaded = load(db_path, snapshot_json)?;
        loaded.require(from)?;
        loaded.require(to)?;
        match loaded.graph.path(from, to) {
            None => Ok("null".to_string()),
            Some(steps) => {
                let nodes: Vec<Value> = [from, to]
                    .iter()
                    .filter_map(|id| loaded.graph.get(id))
                    .map(|n| serde_json::to_value(n).expect("a node always serializes"))
                    .collect();
                let edges: Vec<Value> = steps
                    .iter()
                    .filter_map(|step| {
                        loaded
                            .graph
                            .edges()
                            .find(|e| e.from == step.from && e.to == step.to && e.kind == step.kind)
                    })
                    .map(|e| serde_json::to_value(e).expect("an edge always serializes"))
                    .collect();
                to_json_string(&json!({ "path": steps, "nodes": nodes, "edges": edges }))
            }
        }
    })
}

/// `codetopo_neighborhood` — `{node, depth, nodes, edges}`.
#[extendr]
fn ct_neighborhood(db_path: &str, snapshot_json: &str, id: &str, depth: i32) -> String {
    boundary(|| {
        let loaded = load(db_path, snapshot_json)?;
        loaded.require(id)?;
        let sub = loaded.graph.neighborhood(id, depth.max(0) as u32);
        to_json_string(&json!({
            "node": id,
            "depth": depth,
            "nodes": sub.nodes,
            "edges": sub.edges,
        }))
    })
}

/// `codetopo_diagnostics` — the diagnostics recorded at index time.
/// Diagnostics live in the store, so a snapshot-born handle (which
/// never went through an index) reports an empty record rather than
/// inventing one.
#[extendr]
fn ct_diagnostics(db_path: &str, snapshot_json: &str) -> String {
    boundary(|| {
        let loaded = load(db_path, snapshot_json)?;
        match &loaded.db_path {
            Some(db) => {
                let value = codetopo_cli::diagnostics_value(Path::new(db)).map_err(cli_error)?;
                to_json_string(&value)
            }
            None => to_json_string(&json!({ "total": 0, "diagnostics": [] })),
        }
    })
}

/// `codetopo_rank` — the CLI library's rank value for the metric named
/// by `by`. An unknown metric is an `invalid_param` error naming the
/// accepted vocabulary.
#[extendr]
fn ct_rank(db_path: &str, snapshot_json: &str, by: &str, limit: i32) -> String {
    boundary(|| {
        let loaded = load(db_path, snapshot_json)?;
        let metric = RankMetric::parse(by).ok_or_else(|| {
            (
                "invalid_param".to_string(),
                format!(
                    "unknown rank metric '{by}' (expected one of: fan_in, fan_out, blast_radius, complexity, pagerank, betweenness, closeness, eigenvector, katz)"
                ),
            )
        })?;
        to_json_string(&codetopo_cli::rank_value(
            &loaded.graph,
            metric,
            limit.max(0) as usize,
        ))
    })
}

/// `codetopo_metrics` — the CLI library's metrics value.
#[extendr]
fn ct_metrics(db_path: &str, snapshot_json: &str) -> String {
    boundary(|| {
        let loaded = load(db_path, snapshot_json)?;
        to_json_string(&codetopo_cli::metrics_value(&loaded.graph))
    })
}

/// `codetopo_communities` — the CLI library's communities value.
#[extendr]
fn ct_communities(db_path: &str, snapshot_json: &str, limit: i32) -> String {
    boundary(|| {
        let loaded = load(db_path, snapshot_json)?;
        to_json_string(&codetopo_cli::communities_value(
            &loaded.graph,
            limit.max(0) as usize,
        ))
    })
}

/// `codetopo_topo_sort` — the CLI library's topo-sort value.
#[extendr]
fn ct_topo_sort(db_path: &str, snapshot_json: &str) -> String {
    boundary(|| {
        let loaded = load(db_path, snapshot_json)?;
        to_json_string(&codetopo_cli::topo_sort_value(&loaded.graph))
    })
}

/// `codetopo_render` — the shared layout-spec answer (`layered` v1 /
/// `force` v2); an unknown layout is an `invalid_param` error.
#[extendr]
fn ct_render(db_path: &str, snapshot_json: &str, layout: &str) -> String {
    boundary(|| {
        let loaded = load(db_path, snapshot_json)?;
        let value = codetopo_cli::render_value(&loaded.graph, layout)
            .map_err(|msg| ("invalid_param".to_string(), msg))?;
        to_json_string(&value)
    })
}

/// `codetopo_history` — change-history measurements from a local
/// working tree's git log. With no repo (or no usable history) the
/// answer is the unmeasured report with its reason — absence is an
/// answer.
#[extendr]
fn ct_history(
    db_path: &str,
    snapshot_json: &str,
    repo_path: &str,
    since: &str,
    min_support: i32,
) -> String {
    boundary(|| {
        let loaded = load(db_path, snapshot_json)?;
        let since_opt = opt(since);
        let (input, reason) = match opt(repo_path) {
            Some(repo) => {
                match codetopo_cli::repo_history_input(
                    &loaded.graph,
                    Path::new(&repo),
                    since_opt.as_deref(),
                ) {
                    Ok(input) => (Some(input), String::new()),
                    Err(reason) => (None, reason),
                }
            }
            None => (
                None,
                "no working tree supplied: history is measured from a local repository; an index alone carries no history"
                    .to_string(),
            ),
        };
        to_json_string(&codetopo_cli::history_value(
            &loaded.graph,
            input,
            min_support.max(0) as usize,
            &reason,
        ))
    })
}

/// `codetopo_diff` — the change-impact diff against another handle,
/// in the CLI library's shared diff shape.
#[extendr]
fn ct_diff(db_path: &str, snapshot_json: &str, other_db_path: &str, other_snapshot_json: &str) -> String {
    boundary(|| {
        let loaded = load(db_path, snapshot_json)?;
        let other = load(other_db_path, other_snapshot_json)?;
        to_json_string(&codetopo_cli::diff_value(&loaded.graph, &other.graph))
    })
}

/// `codetopo_snapshot` — the portable frozen form: the `Snapshot`
/// struct itself, bound to the current audit head.
#[extendr]
fn ct_snapshot(db_path: &str, snapshot_json: &str) -> String {
    boundary(|| {
        let loaded = load(db_path, snapshot_json)?;
        let snap = Snapshot::from_graph(&loaded.graph, &loaded.audit_entries_fresh());
        to_json_string(&serde_json::to_value(snap).map_err(|e| {
            (
                "internal".to_string(),
                format!("snapshot does not serialize: {e}"),
            )
        })?)
    })
}

/// `codetopo_verify` — audit-chain verification in the MCP shape
/// `{entries, chain_valid, head}`. Database handles verify the
/// store's chain; snapshot-born handles verify the chain the
/// snapshot carries.
#[extendr]
fn ct_verify(db_path: &str, snapshot_json: &str) -> String {
    boundary(|| {
        let loaded = load(db_path, snapshot_json)?;
        if let Some(db) = &loaded.db_path {
            let store = Store::open(Path::new(db))
                .map_err(|e| ("store_error".to_string(), format!("store error: {e}")))?;
            let audit = store
                .load_audit()
                .map_err(|e| ("store_error".to_string(), format!("store error: {e}")))?;
            return to_json_string(&json!({
                "entries": audit.len(),
                "chain_valid": audit.verify(),
                "head": audit.head(),
            }));
        }
        let audit = AuditLog::from_snapshot_entries(&loaded.audit_entries);
        to_json_string(&json!({
            "entries": audit.len(),
            "chain_valid": audit.verify(),
            "head": audit.head(),
        }))
    })
}

// Macro to generate exports.
// This ensures exported functions are registered with R.
// See corresponding C code in `entrypoint.c`.
extendr_module! {
    mod codetopo;
    fn ct_core_version;
    fn ct_schema_version;
    fn ct_describe_schema;
    fn ct_index;
    fn ct_restore;
    fn ct_stats;
    fn ct_search_symbols;
    fn ct_get_symbol;
    fn ct_closure;
    fn ct_path;
    fn ct_neighborhood;
    fn ct_diagnostics;
    fn ct_rank;
    fn ct_metrics;
    fn ct_communities;
    fn ct_topo_sort;
    fn ct_render;
    fn ct_history;
    fn ct_diff;
    fn ct_snapshot;
    fn ct_verify;
}
