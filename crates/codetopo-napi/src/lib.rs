// Copyright (C) 2026 2351485 ALBERTA INC.
// SPDX-License-Identifier: AGPL-3.0-or-later
// Part of codetopo — Owned by 2351485 ALBERTA INC. (https://nrupalakolkar.com) · Built with Muse by Meta (https://muse.ai/join).

//! codetopo-napi — the JavaScript surface (Diaspora Wave 1a).
//!
//! A thin napi-rs transport over [`codetopo_cli`]'s library, exactly as
//! dossier §1.3 prescribes: the boundary exchanges CODE_SCHEMA JSON.
//! Indexing calls `codetopo_cli::index_repo`; every query returns the
//! same `serde_json::Value` the CLI library builds for the CLI / HTTP /
//! MCP surfaces, so the JS answers are those answers by construction —
//! parity is inherited, not re-implemented. Where the CLI library only
//! ships a print form (stats, the id-list closures), this crate rebuilds
//! that print form's JSON shape field-for-field from the same core
//! calls.
//!
//! All values cross the boundary as JSON text; the hand-written JS
//! layer (`codetopo.js`) parses them into plain objects. Errors cross
//! as a JSON-encoded reason (`{"code", "message"}`) which the JS layer
//! rethrows as an `Error` carrying the stable `code`.
//!
//! Heavy operations (index, rank, metrics, communities, history) are
//! [`AsyncTask`]s on the libuv pool: they return Promises and never
//! block the Node event loop.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use codetopo_cli::CliError;
use codetopo_core::snapshot::SnapshotAuditEntry;
use codetopo_core::{Graph, RankMetric, Snapshot};
use codetopo_store::{AuditLog, Store};
use napi::bindgen_prelude::*;
use napi::Task;
use napi_derive::napi;
use serde_json::{json, Value};

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Build a napi error whose reason is the JSON error envelope the JS
/// layer decodes into an `Error` with a stable `code`.
fn coded_error(code: &str, message: impl Into<String>) -> Error {
    Error::from_reason(json!({ "code": code, "message": message.into() }).to_string())
}

fn cli_error(err: CliError) -> Error {
    match &err {
        CliError::UnknownId(id) => {
            coded_error("unknown_symbol", format!("unknown symbol '{id}': not present in the graph"))
        }
        CliError::Usage(msg) if msg.contains("unsupported") && msg.contains("schema_version") => {
            coded_error("unsupported_schema_version", msg.clone())
        }
        CliError::Usage(msg) => coded_error("invalid_param", msg.clone()),
        CliError::Store(e) => coded_error("store_error", format!("store error: {e}")),
        CliError::Io(e) => coded_error("io_error", format!("io error: {e}")),
        CliError::Json(e) => coded_error("invalid_json", format!("json error: {e}")),
    }
}

fn unknown_symbol(id: &str) -> Error {
    coded_error(
        "unknown_symbol",
        format!("unknown symbol '{id}': not present in the graph"),
    )
}

fn to_json_string(value: &Value) -> Result<String> {
    serde_json::to_string(value)
        .map_err(|e| coded_error("internal", format!("answer does not serialize: {e}")))
}

fn parse_snapshot(snapshot_json: &str) -> Result<Snapshot> {
    let snap = Snapshot::from_json(snapshot_json)
        .map_err(|e| coded_error("invalid_json", format!("snapshot does not parse: {e}")))?;
    snap.check_schema_version()
        .map_err(|msg| coded_error("unsupported_schema_version", msg))?;
    Ok(snap)
}

// ---------------------------------------------------------------------------
// Provenance + schema
// ---------------------------------------------------------------------------

/// The version of the bound `codetopo-core` crate, captured by build.rs
/// from that crate's own manifest.
#[napi]
pub fn core_version() -> String {
    env!("CODETOPO_CORE_VERSION").to_string()
}

/// The CODE_SCHEMA version the bound core emits — the core's own
/// constant, never a restated literal.
#[napi]
pub fn schema_version() -> String {
    codetopo_core::snapshot::SCHEMA_VERSION.to_string()
}

/// `codetopo_describe_schema`: the schema description every surface serves.
#[napi]
pub fn describe_schema() -> Result<String> {
    to_json_string(&codetopo_cli::describe_schema())
}

// ---------------------------------------------------------------------------
// The graph handle
// ---------------------------------------------------------------------------

/// A loaded graph plus the context its queries need: the database it
/// came from (diagnostics and the audit chain live in the store, not
/// the graph) and the audit entries a snapshot of it must carry.
/// Snapshot-born handles have no database; their audit entries are the
/// snapshot's own.
#[napi]
// codeql[rust/access-invalid-pointer] -- False positive (alert #74,
// diaspora program 2026-11-09): the flagged dereference is in the
// object-wrap code napi-derive generates for this type -- casting the
// JS object's internal pointer back to the Rust struct, the standard
// napi-rs class pattern. The struct itself holds only Arc, Option and
// Vec; no pointer is dereferenced in code written here. Suppression
// recorded in-tree so the reasoning travels with the code; the
// release gate can audit it in the code-scanning UI.
pub struct GraphHandle {
    graph: Arc<Graph>,
    db_path: Option<String>,
    audit_entries: Vec<SnapshotAuditEntry>,
}

impl GraphHandle {
    fn from_graph(
        graph: Graph,
        db_path: Option<String>,
        audit_entries: Vec<SnapshotAuditEntry>,
    ) -> Self {
        GraphHandle {
            graph: Arc::new(graph),
            db_path,
            audit_entries,
        }
    }

    fn require(&self, id: &str) -> Result<()> {
        if self.graph.get(id).is_some() {
            Ok(())
        } else {
            Err(unknown_symbol(id))
        }
    }

    fn closure(&self, relation: &str, id: &str, limit: u32, expand: bool) -> Result<String> {
        self.require(id)?;
        let limit = limit as usize;
        let members: Vec<String> = match relation {
            "descendants" => self.graph.descendants(id),
            "ancestors" => self.graph.ancestors(id),
            _ => self.graph.blast_radius(id),
        };
        let total = members.len();
        let shown: &[String] = &members[..total.min(limit)];
        // Field-for-field the CLI library's JSON id-list form
        // (`print_id_list_json`); with `expand`, the shown members are
        // the full node objects, as on the MCP surface.
        let nodes: Vec<Value> = if expand {
            shown
                .iter()
                .filter_map(|mid| self.graph.get(mid))
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

    fn audit_entries_fresh(&self) -> Vec<SnapshotAuditEntry> {
        // A database handle re-reads the chain so a snapshot taken
        // after a re-index binds the current head (the MCP snapshot
        // tool does the same); a snapshot-born handle replays its own.
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

/// Open an index database into a handle (`Codetopo.open` underneath).
#[napi]
pub fn open_handle(db_path: String) -> Result<GraphHandle> {
    let path = Path::new(&db_path);
    if !path.is_file() {
        return Err(coded_error(
            "unknown_index",
            format!("unknown index: no database file at '{db_path}'"),
        ));
    }
    let graph = codetopo_cli::load_graph_from_db(path).map_err(cli_error)?;
    let audit_entries = Store::open(path)
        .and_then(|store| store.load_audit())
        .map(|audit| audit.snapshot_entries())
        .unwrap_or_default();
    Ok(GraphHandle::from_graph(graph, Some(db_path), audit_entries))
}

/// Build a handle directly from snapshot JSON (`fromSnapshot`).
/// The CODE_SCHEMA §8 major-version rule is enforced loudly.
#[napi]
pub fn handle_from_snapshot(snapshot_json: String) -> Result<GraphHandle> {
    let snap = parse_snapshot(&snapshot_json)?;
    let audit_entries = snap.audit_log.clone();
    Ok(GraphHandle::from_graph(snap.to_graph(), None, audit_entries))
}

/// Restore snapshot JSON into a database at `db_path` — the full
/// verification gate, as on the CLI: schema major, audit chain
/// recomputing to the declared head. A snapshot with no audit trail is
/// refused (there is nothing to verify it against). Returns a JSON
/// report `{nodes, edges, verified}`.
#[napi]
pub fn restore_snapshot(snapshot_json: String, db_path: String) -> Result<String> {
    let snap = parse_snapshot(&snapshot_json)?;
    if snap.audit_log.is_empty() {
        return Err(coded_error(
            "unverified_snapshot",
            "snapshot carries no audit trail — nothing to verify it against",
        ));
    }
    let audit = AuditLog::from_snapshot_entries(&snap.audit_log);
    if !audit.verify() || audit.head() != snap.audit_head {
        return Err(coded_error(
            "audit_chain_invalid",
            "snapshot audit chain does not verify against its declared head",
        ));
    }
    let graph = snap.to_graph();
    let nodes = graph.node_count();
    let edges = graph.edge_count();
    let mut store = Store::open(Path::new(&db_path))
        .map_err(|e| coded_error("store_error", format!("store error: {e}")))?;
    store
        .replace_graph(&graph)
        .map_err(|e| coded_error("store_error", format!("store error: {e}")))?;
    store
        .save_audit(&audit)
        .map_err(|e| coded_error("store_error", format!("store error: {e}")))?;
    to_json_string(&json!({ "nodes": nodes, "edges": edges, "verified": true }))
}

#[napi]
impl GraphHandle {
    /// The database this handle was opened from, when it has one.
    #[napi(getter)]
    pub fn db_path(&self) -> Option<String> {
        self.db_path.clone()
    }

    /// `codetopo_stats` — the CLI library's JSON stats shape
    /// (`{nodes: {total, by_kind}, edges: {total, by_kind}}`).
    #[napi]
    pub fn stats(&self) -> Result<String> {
        let mut node_kinds = serde_json::Map::new();
        for kind in codetopo_cli::ALL_NODE_KINDS {
            let count = self.graph.nodes().filter(|n| n.kind == kind).count();
            if count > 0 {
                node_kinds.insert(codetopo_cli::node_kind_name(kind).to_string(), count.into());
            }
        }
        let mut edge_kinds = serde_json::Map::new();
        for kind in codetopo_cli::ALL_EDGE_KINDS {
            let count = self.graph.edge_count_by_kind(kind);
            if count > 0 {
                edge_kinds.insert(kind.as_str().to_string(), count.into());
            }
        }
        to_json_string(&json!({
            "nodes": { "total": self.graph.node_count(), "by_kind": node_kinds },
            "edges": { "total": self.graph.edge_count(), "by_kind": edge_kinds },
        }))
    }

    /// `codetopo_search_symbols` — `{query, total, results}`.
    #[napi]
    pub fn search_symbols(&self, query: String, limit: u32) -> Result<String> {
        let hits = self.graph.search_symbols(&query, limit as usize);
        to_json_string(&json!({ "query": query, "total": hits.len(), "results": hits }))
    }

    /// `codetopo_get_symbol` — the symbol dossier, serialized by the
    /// core type itself.
    #[napi]
    pub fn get_symbol(&self, id: String) -> Result<String> {
        let dossier = self
            .graph
            .dossier(&id)
            .ok_or_else(|| unknown_symbol(&id))?;
        to_json_string(
            &serde_json::to_value(dossier)
                .map_err(|e| coded_error("internal", format!("dossier does not serialize: {e}")))?,
        )
    }

    /// `codetopo_descendants` — transitive closure over calls + imports.
    #[napi]
    pub fn descendants(&self, id: String, limit: u32, expand: bool) -> Result<String> {
        self.closure("descendants", &id, limit, expand)
    }

    /// `codetopo_ancestors` — the reverse closure.
    #[napi]
    pub fn ancestors(&self, id: String, limit: u32, expand: bool) -> Result<String> {
        self.closure("ancestors", &id, limit, expand)
    }

    /// `codetopo_blast_radius` — transitive dependents over the
    /// breakage-propagating edge kinds.
    #[napi]
    pub fn blast_radius(&self, id: String, limit: u32, expand: bool) -> Result<String> {
        self.closure("blast_radius", &id, limit, expand)
    }

    /// `codetopo_path` — the shortest directed typed path. JSON `null`
    /// when both endpoints exist but no directed path joins them (a
    /// path is never synthesized by reversing edges); an unknown
    /// endpoint is an `unknown_symbol` error.
    #[napi]
    pub fn path(&self, from: String, to: String) -> Result<String> {
        self.require(&from)?;
        self.require(&to)?;
        match self.graph.path(&from, &to) {
            None => Ok("null".to_string()),
            Some(steps) => {
                let nodes: Vec<Value> = [&from, &to]
                    .iter()
                    .filter_map(|id| self.graph.get(id))
                    .map(|n| serde_json::to_value(n).expect("a node always serializes"))
                    .collect();
                let edges: Vec<Value> = steps
                    .iter()
                    .filter_map(|step| {
                        self.graph
                            .edges()
                            .find(|e| e.from == step.from && e.to == step.to && e.kind == step.kind)
                    })
                    .map(|e| serde_json::to_value(e).expect("an edge always serializes"))
                    .collect();
                to_json_string(&json!({ "path": steps, "nodes": nodes, "edges": edges }))
            }
        }
    }

    /// `codetopo_neighborhood` — `{node, depth, nodes, edges}`.
    #[napi]
    pub fn neighborhood(&self, id: String, depth: u32) -> Result<String> {
        self.require(&id)?;
        let sub = self.graph.neighborhood(&id, depth);
        to_json_string(&json!({
            "node": id,
            "depth": depth,
            "nodes": sub.nodes,
            "edges": sub.edges,
        }))
    }

    /// `codetopo_diagnostics` — the diagnostics recorded at index time,
    /// in the CLI library's `{total, diagnostics}` shape. Diagnostics
    /// live in the store, so a snapshot-born handle (which never went
    /// through an index) reports an empty record rather than inventing
    /// one.
    #[napi]
    pub fn diagnostics(&self) -> Result<String> {
        match &self.db_path {
            Some(db) => {
                let value =
                    codetopo_cli::diagnostics_value(Path::new(db)).map_err(cli_error)?;
                to_json_string(&value)
            }
            None => to_json_string(&json!({ "total": 0, "diagnostics": [] })),
        }
    }

    /// `codetopo_topo_sort` — the CLI library's topo-sort value.
    #[napi]
    pub fn topo_sort(&self) -> Result<String> {
        to_json_string(&codetopo_cli::topo_sort_value(&self.graph))
    }

    /// `codetopo_render` — the shared layout-spec answer (`layered`
    /// v1 / `force` v2); an unknown layout is an `invalid_param` error.
    #[napi]
    pub fn render(&self, layout: String) -> Result<String> {
        let value = codetopo_cli::render_value(&self.graph, &layout)
            .map_err(|msg| coded_error("invalid_param", msg))?;
        to_json_string(&value)
    }

    /// `codetopo_diff` — the change-impact diff against another handle,
    /// in the CLI library's shared diff shape.
    #[napi]
    pub fn diff(&self, other: &GraphHandle) -> Result<String> {
        to_json_string(&codetopo_cli::diff_value(&self.graph, &other.graph))
    }

    /// `codetopo_snapshot` — the portable frozen form: the `Snapshot`
    /// struct itself, bound to the current audit head.
    #[napi]
    pub fn snapshot(&self) -> Result<String> {
        let snap = Snapshot::from_graph(&self.graph, &self.audit_entries_fresh());
        to_json_string(
            &serde_json::to_value(snap)
                .map_err(|e| coded_error("internal", format!("snapshot does not serialize: {e}")))?,
        )
    }

    /// `codetopo_verify` — audit-chain verification in the MCP shape
    /// `{entries, chain_valid, head}`. Database handles verify the
    /// store's chain; snapshot-born handles verify the chain the
    /// snapshot carries.
    #[napi]
    pub fn verify(&self) -> Result<String> {
        if let Some(db) = &self.db_path {
            let store = Store::open(Path::new(db))
                .map_err(|e| coded_error("store_error", format!("store error: {e}")))?;
            let audit = store
                .load_audit()
                .map_err(|e| coded_error("store_error", format!("store error: {e}")))?;
            return to_json_string(&json!({
                "entries": audit.len(),
                "chain_valid": audit.verify(),
                "head": audit.head(),
            }));
        }
        let audit = AuditLog::from_snapshot_entries(&self.audit_entries);
        to_json_string(&json!({
            "entries": audit.len(),
            "chain_valid": audit.verify(),
            "head": audit.head(),
        }))
    }

    /// `codetopo_rank` (async) — the CLI library's rank value for the
    /// metric named by `by`. An unknown metric is an `invalid_param`
    /// error naming the accepted vocabulary.
    #[napi]
    pub fn rank(&self, by: String, limit: u32) -> AsyncTask<RankTask> {
        AsyncTask::new(RankTask {
            graph: self.graph.clone(),
            by,
            limit: limit as usize,
        })
    }

    /// `codetopo_metrics` (async) — the CLI library's metrics value.
    #[napi]
    pub fn metrics(&self) -> AsyncTask<MetricsTask> {
        AsyncTask::new(MetricsTask {
            graph: self.graph.clone(),
        })
    }

    /// `codetopo_communities` (async) — the CLI library's communities
    /// value.
    #[napi]
    pub fn communities(&self, limit: u32) -> AsyncTask<CommunitiesTask> {
        AsyncTask::new(CommunitiesTask {
            graph: self.graph.clone(),
            limit: limit as usize,
        })
    }

    /// `codetopo_history` (async) — change-history measurements from a
    /// local working tree's git log, in the CLI library's history
    /// shape. With no repo (or no usable history) the answer is the
    /// unmeasured report with its reason — absence is an answer.
    #[napi]
    pub fn history(
        &self,
        repo_path: Option<String>,
        since: Option<String>,
        min_support: u32,
    ) -> AsyncTask<HistoryTask> {
        AsyncTask::new(HistoryTask {
            graph: self.graph.clone(),
            repo_path,
            since,
            min_support: min_support as usize,
        })
    }
}

// ---------------------------------------------------------------------------
// Async tasks (heavy work off the event loop)
// ---------------------------------------------------------------------------

/// `codetopo_index` — walk, extract, build, persist.
pub struct IndexTask {
    repo_path: String,
    db_path: String,
    package: String,
}

impl Task for IndexTask {
    type Output = String;
    type JsValue = String;

    fn compute(&mut self) -> Result<Self::Output> {
        let repo = PathBuf::from(&self.repo_path);
        let db = PathBuf::from(&self.db_path);
        let report = codetopo_cli::index_repo(&repo, &db, &self.package, None, None)
            .map_err(cli_error)?;
        to_json_string(&json!({
            "package": self.package,
            "repo_path": self.repo_path,
            "db_path": self.db_path,
            "files_indexed": report.files_indexed,
            "files_failed": report.files_failed,
            "nodes": report.nodes,
            "edges": report.edges,
            "diagnostics": report.diagnostics,
        }))
    }

    fn resolve(&mut self, _env: Env, output: Self::Output) -> Result<Self::JsValue> {
        Ok(output)
    }
}

/// Index a repository (async). The package defaults to the repository
/// directory's name, as on the other surfaces.
#[napi]
pub fn index_repo_async(
    repo_path: String,
    db_path: String,
    package: Option<String>,
) -> AsyncTask<IndexTask> {
    let package = package.unwrap_or_else(|| {
        codetopo_cli::default_package_name(Path::new(&repo_path))
    });
    AsyncTask::new(IndexTask {
        repo_path,
        db_path,
        package,
    })
}

pub struct RankTask {
    graph: Arc<Graph>,
    by: String,
    limit: usize,
}

impl Task for RankTask {
    type Output = String;
    type JsValue = String;

    fn compute(&mut self) -> Result<Self::Output> {
        let metric = RankMetric::parse(&self.by).ok_or_else(|| {
            coded_error(
                "invalid_param",
                format!(
                    "unknown rank metric '{}' (expected one of: fan_in, fan_out, blast_radius, complexity, pagerank, betweenness, closeness, eigenvector, katz)",
                    self.by
                ),
            )
        })?;
        to_json_string(&codetopo_cli::rank_value(&self.graph, metric, self.limit))
    }

    fn resolve(&mut self, _env: Env, output: Self::Output) -> Result<Self::JsValue> {
        Ok(output)
    }
}

pub struct MetricsTask {
    graph: Arc<Graph>,
}

impl Task for MetricsTask {
    type Output = String;
    type JsValue = String;

    fn compute(&mut self) -> Result<Self::Output> {
        to_json_string(&codetopo_cli::metrics_value(&self.graph))
    }

    fn resolve(&mut self, _env: Env, output: Self::Output) -> Result<Self::JsValue> {
        Ok(output)
    }
}

pub struct CommunitiesTask {
    graph: Arc<Graph>,
    limit: usize,
}

impl Task for CommunitiesTask {
    type Output = String;
    type JsValue = String;

    fn compute(&mut self) -> Result<Self::Output> {
        to_json_string(&codetopo_cli::communities_value(&self.graph, self.limit))
    }

    fn resolve(&mut self, _env: Env, output: Self::Output) -> Result<Self::JsValue> {
        Ok(output)
    }
}

pub struct HistoryTask {
    graph: Arc<Graph>,
    repo_path: Option<String>,
    since: Option<String>,
    min_support: usize,
}

impl Task for HistoryTask {
    type Output = String;
    type JsValue = String;

    fn compute(&mut self) -> Result<Self::Output> {
        // The MCP surface's history logic verbatim: a supplied repo is
        // read; any failure to read becomes the unmeasured reason.
        let (input, reason) = match &self.repo_path {
            Some(repo) => {
                match codetopo_cli::repo_history_input(
                    &self.graph,
                    Path::new(repo),
                    self.since.as_deref(),
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
            &self.graph,
            input,
            self.min_support,
            &reason,
        ))
    }

    fn resolve(&mut self, _env: Env, output: Self::Output) -> Result<Self::JsValue> {
        Ok(output)
    }
}
