// Copyright (C) 2026 Nrupal Akolkar
// SPDX-License-Identifier: AGPL-3.0-or-later
// Part of codetopo — Owned by Nrupal Akolkar · Built with Muse by Meta.

//! The eight request handlers of the Phase 2 API (design §3).
//!
//! This module owns no graph logic. Every route is a transport rename of a
//! command the CLI already runs: the same `codetopo_core` query answers it, and
//! the same `codetopo_store` read backs it. That is deliberate — §3.2 makes
//! CLI/API parity an exit criterion, and shared code is the only way to make
//! parity true by construction rather than by two implementations that happen
//! to agree today.
//!
//! Three rules shape the code below:
//!
//! * **Aggregate first.** A closure response always carries `total`, the full
//!   size of the answer, before any list. An agent learns the shape of the
//!   result from one integer and decides whether it can afford the list; a
//!   response that is only a list forces it to page before it knows the size.
//! * **Blocking work leaves the async runtime.** `rusqlite` and tree walking
//!   are synchronous, and §6 requires every db-touching handler to run inside
//!   `spawn_blocking` so a slow query cannot stall the accept loop.
//! * **Errors are flat and coded.** Every failure path returns [`ApiError`],
//!   whose body is `{"error": "<code>", "message": "…"}` (see [`crate::error`]
//!   for why the body is flat rather than nested).

use std::collections::BTreeMap;
use std::path::{Path as FsPath, PathBuf};
use std::sync::Arc;

use axum::extract::{Path as UrlPath, Query, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use codetopo_cli::{index_repo, load_graph_from_db, render_path, CliError};
use codetopo_core::graph::{Graph, PathStep};
use codetopo_core::schema::{EdgeKind, NodeKind};
use codetopo_core::snapshot::Snapshot;
use codetopo_store::Store;

use crate::config::Config;
use crate::error::ApiError;

/// Default `limit` for closure queries, matching the CLI's `--limit` default.
const DEFAULT_LIMIT: usize = 20;

/// Upper bound on `limit` (§3.1). A closure over a whole repository can hold
/// hundreds of thousands of ids, and an unbounded `limit` would let one request
/// allocate the whole graph's id set — the id list is the response, so the cap
/// is the only thing between a query and an out-of-memory.
const MAX_LIMIT: usize = 1000;

/// Shared, immutable server state.
///
/// Only the resolved configuration: keys and the metering log are owned by the
/// auth layer, which is applied around the router rather than inside it, so a
/// handler physically cannot reach a secret.
#[derive(Debug, Clone)]
pub struct AppState {
    config: Arc<Config>,
}

impl AppState {
    /// Wraps a resolved [`Config`] for use as axum state.
    ///
    /// Takes the `Arc` the process already holds rather than a `Config` by
    /// value, so the state and the configuration `main` logs at startup are
    /// provably the same value and building a router in a test does not require
    /// cloning one.
    pub fn new(config: Arc<Config>) -> Self {
        AppState { config }
    }

    /// Database path for one index id (§6: `<data_dir>/<id>.db`).
    fn db_path(&self, index_id: &str) -> PathBuf {
        self.config.index_db_path(index_id)
    }
}

/// Build the unauthenticated route table.
///
/// [`crate::auth::metered`] wraps the result; this function only decides which
/// paths exist, so that auth coverage is a property of the wrapper rather than
/// something each route has to opt into.
pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/v1/index", post(index))
        .route("/v1/graphs/:id/descendants", get(descendants))
        .route("/v1/graphs/:id/ancestors", get(ancestors))
        .route("/v1/graphs/:id/blast-radius", get(blast_radius))
        .route("/v1/graphs/:id/path", get(path))
        .route("/v1/graphs/:id/stats", get(stats))
        .route("/v1/graphs/:id/verify", get(verify))
        .route("/v1/graphs/:id/snapshot", get(snapshot))
        .with_state(state)
}

// ---------------------------------------------------------------------------
// POST /v1/index
// ---------------------------------------------------------------------------

/// Body of `POST /v1/index`.
///
/// Every field is optional at the type level so that a *missing* field is
/// reported as `invalid_repo_path` with a message naming it, rather than as
/// axum's generic JSON rejection, which arrives before any handler code runs.
#[derive(Debug, Default, Deserialize)]
struct IndexRequest {
    repo_path: Option<String>,
    package: Option<String>,
}

/// Package name recorded for the index when the request omits one.
///
/// The CLI has a `--package` flag with a caller-supplied value and no default
/// worth inheriting here; `"unknown"` keeps the field populated (a node of kind
/// `package` whose name is empty would be a lie) while being obviously not a
/// real name.
const UNKNOWN_PACKAGE: &str = "unknown";

/// `POST /v1/index` — index a repository tree into a new index id.
///
/// Walks the tree, writes `<data_dir>/<index_id>.db`, and returns the CLI's own
/// `IndexReport` fields, so API stats and CLI stats come from one computation.
async fn index(
    State(state): State<AppState>,
    Json(request): Json<IndexRequest>,
) -> Result<Json<Value>, ApiError> {
    let repo = request
        .repo_path
        .as_deref()
        .map(str::trim)
        .filter(|raw| !raw.is_empty())
        .ok_or_else(|| ApiError::invalid_repo_path("repo_path is required"))?;
    let repo = PathBuf::from(repo);
    if !repo.is_dir() {
        return Err(ApiError::invalid_repo_path(&format!(
            "not a directory: {}",
            repo.display()
        )));
    }

    let package = request
        .package
        .as_deref()
        .map(str::trim)
        .filter(|raw| !raw.is_empty())
        .unwrap_or(UNKNOWN_PACKAGE)
        .to_string();

    // §6: a UUID v4 rather than a repo name, so re-indexing the same tree does
    // not clobber the previous index and no customer directory layout leaks
    // into a URL or a log line.
    let index_id = uuid::Uuid::new_v4().to_string();
    let db = state.db_path(&index_id);

    let report = tokio::task::spawn_blocking(move || index_repo(&repo, &db, &package, None, None))
        .await
        .map_err(|err| ApiError::internal(format!("index task failed: {err}")))?
        .map_err(|err| ApiError::internal(format!("cannot index repository: {err}")))?;

    Ok(Json(json!({
        "index_id": index_id,
        "files_indexed": report.files_indexed,
        "files_failed": report.files_failed,
        "nodes": report.nodes,
        "edges": report.edges,
        "diagnostics": report.diagnostics,
    })))
}

// ---------------------------------------------------------------------------
// Closure routes: descendants / ancestors / blast-radius
// ---------------------------------------------------------------------------

/// Raw query string for a closure route.
///
/// `expand` and `limit` are read as strings so a bad value is reported as
/// `invalid_param` naming the parameter, instead of being turned into a
/// deserialization error before a handler sees it.
#[derive(Debug, Default, Deserialize)]
struct ClosureQuery {
    node: Option<String>,
    expand: Option<String>,
    limit: Option<String>,
}

/// Validated closure parameters.
#[derive(Debug)]
struct ClosureParams {
    node: String,
    expand: bool,
    limit: usize,
}

/// Which direction a closure walks.
#[derive(Debug, Clone, Copy)]
enum Direction {
    /// Outgoing edges: what this node affects.
    Downstream,
    /// Incoming edges: what depends on this node.
    Upstream,
}

/// Parse `node`, `expand`, and `limit` per §3.1.
///
/// Absent and empty are treated alike, so `?limit=` means "use the default"
/// rather than "reject", which keeps a client that templates optional query
/// parameters from failing on the empty string.
fn closure_params(query: &ClosureQuery) -> Result<ClosureParams, ApiError> {
    let node = query
        .node
        .as_deref()
        .map(str::trim)
        .filter(|raw| !raw.is_empty())
        .ok_or_else(|| ApiError::missing_param("node"))?
        .to_string();

    let expand = match query.expand.as_deref().map(str::trim) {
        None | Some("") => false,
        Some("true") => true,
        Some("false") => false,
        Some(other) => {
            return Err(ApiError::invalid_param(
                "expand",
                &format!("expected 'true' or 'false', got '{other}'"),
            ))
        }
    };

    let limit = match query.limit.as_deref().map(str::trim) {
        None | Some("") => DEFAULT_LIMIT,
        Some(raw) => {
            let parsed: usize = raw.parse().map_err(|_| {
                ApiError::invalid_param("limit", &format!("expected an integer, got '{raw}'"))
            })?;
            if parsed == 0 || parsed > MAX_LIMIT {
                return Err(ApiError::invalid_param(
                    "limit",
                    &format!("must be 1..={MAX_LIMIT}, got {parsed}"),
                ));
            }
            parsed
        }
    };

    Ok(ClosureParams {
        node,
        expand,
        limit,
    })
}

/// Aggregate-first closure response.
///
/// `total` is the size of the *whole* closure; `nodes` is a possibly-truncated
/// view of it. `truncated` says which of the two the client got, so a client
/// never has to infer it by comparing two numbers.
#[derive(Debug, Serialize)]
struct ClosureResponse {
    index_id: String,
    relation: &'static str,
    node: String,
    total: usize,
    expand: bool,
    limit: usize,
    truncated: bool,
    nodes: Vec<String>,
}

/// `GET /v1/graphs/{id}/descendants?node=&expand=&limit=`.
///
/// What this node affects: the transitive closure of its outgoing call and
/// import edges.
async fn descendants(
    State(state): State<AppState>,
    UrlPath(index_id): UrlPath<String>,
    Query(query): Query<ClosureQuery>,
) -> Result<Json<ClosureResponse>, ApiError> {
    closure(state, index_id, query, "descendants", Direction::Downstream).await
}

/// `GET /v1/graphs/{id}/ancestors?node=&expand=&limit=`.
///
/// The reverse closure: everything that transitively reaches this node.
async fn ancestors(
    State(state): State<AppState>,
    UrlPath(index_id): UrlPath<String>,
    Query(query): Query<ClosureQuery>,
) -> Result<Json<ClosureResponse>, ApiError> {
    closure(state, index_id, query, "ancestors", Direction::Upstream).await
}

/// `GET /v1/graphs/{id}/blast-radius?node=&expand=&limit=`.
///
/// Breakage propagation: the dependency-side closure a failure of this node
/// would take with it.
async fn blast_radius(
    State(state): State<AppState>,
    UrlPath(index_id): UrlPath<String>,
    Query(query): Query<ClosureQuery>,
) -> Result<Json<ClosureResponse>, ApiError> {
    closure(
        state,
        index_id,
        query,
        "blast_radius",
        Direction::Downstream,
    )
    .await
}

/// Shared body of the three closure routes.
///
/// ## Deviation from §3.1
///
/// §3.1 says `expand=true` returns "the full id list" and, in the same table,
/// that `limit` is the "max ids returned when `expand=true`". Those two
/// statements cannot both hold, and the design gives no tie-breaker. CLI
/// parity (§3.2) is the tie-breaker used here: `print_id_list` caps the list at
/// `limit` by default and returns everything under `--expand`. So `limit` caps
/// the default list and `expand=true` lifts the cap. Nothing is lost either way,
/// because `total` is always the full size — which is the reason the aggregate
/// comes first.
async fn closure(
    state: AppState,
    index_id: String,
    query: ClosureQuery,
    relation: &'static str,
    direction: Direction,
) -> Result<Json<ClosureResponse>, ApiError> {
    let ClosureParams {
        node,
        expand,
        limit,
    } = closure_params(&query)?;
    let db = state.db_path(&index_id);
    let graph = load_graph_blocking(&db, &index_id)?;

    // The walk owns the graph and a copy of the id: `spawn_blocking` needs a
    // `'static` closure, so neither can be borrowed from this frame. The
    // response still reports the caller's `node`, so the copy is the only
    // duplicate of the id that crosses the boundary.
    let walk_node = node.clone();
    let ids = spawn_blocking(move || {
        Ok(match direction {
            Direction::Downstream if relation == "blast_radius" => graph.blast_radius(&walk_node),
            Direction::Downstream => graph.descendants(&walk_node),
            Direction::Upstream => graph.ancestors(&walk_node),
        })
    })
    .await?;

    let total = ids.len();
    let truncated = !expand && total > limit;
    let shown = if expand {
        ids
    } else {
        ids[..total.min(limit)].to_vec()
    };

    Ok(Json(ClosureResponse {
        index_id,
        relation,
        node,
        total,
        expand,
        limit,
        truncated,
        nodes: shown,
    }))
}

// ---------------------------------------------------------------------------
// GET /v1/graphs/{id}/path
// ---------------------------------------------------------------------------

/// Raw query string for `path`. Read as strings for the same reason as
/// [`ClosureQuery`]: a missing `from` must be `missing_param`, not a serde
/// error from the extractor.
#[derive(Debug, Default, Deserialize)]
struct PathQuery {
    from: Option<String>,
    to: Option<String>,
}

/// `GET /v1/graphs/{id}/path?from=&to=` — the shortest typed path between two
/// nodes.
///
/// The response is deliberately the *whole* path rather than a capped prefix:
/// a path is a sequence whose meaning depends on every step, so truncating it
/// would produce a well-formed but false answer.
#[derive(Debug, Serialize)]
struct PathResponse {
    index_id: String,
    from: String,
    to: String,
    steps: Vec<PathStep>,
    rendered: String,
}

async fn path(
    State(state): State<AppState>,
    UrlPath(index_id): UrlPath<String>,
    Query(query): Query<PathQuery>,
) -> Result<Json<PathResponse>, ApiError> {
    let from = required_param(query.from.as_deref(), "from")?;
    let to = required_param(query.to.as_deref(), "to")?;
    let db = state.db_path(&index_id);
    let graph = load_graph_blocking(&db, &index_id)?;

    let steps = graph
        .path(&from, &to)
        .ok_or_else(|| ApiError::no_path(&from, &to))?;
    let rendered = render_path(&steps);

    Ok(Json(PathResponse {
        index_id,
        from,
        to,
        steps,
        rendered,
    }))
}

fn required_param(raw: Option<&str>, name: &str) -> Result<String, ApiError> {
    raw.map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .ok_or_else(|| ApiError::missing_param(name))
}

// ---------------------------------------------------------------------------
// GET /v1/graphs/{id}/stats
// ---------------------------------------------------------------------------

/// Aggregate counts: totals plus per-kind breakdowns.
///
/// The breakdown omits zero kinds, matching `print_stats` (§3.2 parity), and
/// `by_kind` is a map rather than an array so a client can read a single kind
/// without walking a list to find it.
#[derive(Debug, Serialize)]
struct StatsResponse {
    index_id: String,
    nodes: KindCounts,
    edges: KindCounts,
}

#[derive(Debug, Serialize)]
struct KindCounts {
    total: usize,
    by_kind: BTreeMap<String, usize>,
}

async fn stats(
    State(state): State<AppState>,
    UrlPath(index_id): UrlPath<String>,
) -> Result<Json<StatsResponse>, ApiError> {
    let db = state.db_path(&index_id);
    let graph = load_graph_blocking(&db, &index_id)?;

    let mut node_kinds = BTreeMap::new();
    for kind in ALL_NODE_KINDS {
        let count = graph.nodes().filter(|node| node.kind == kind).count();
        if count > 0 {
            node_kinds.insert(node_kind_name(kind).to_string(), count);
        }
    }

    let mut edge_kinds = BTreeMap::new();
    for kind in ALL_EDGE_KINDS {
        let count = graph.edge_count_by_kind(kind);
        if count > 0 {
            edge_kinds.insert(kind.as_str().to_string(), count);
        }
    }

    Ok(Json(StatsResponse {
        index_id,
        nodes: KindCounts {
            total: graph.node_count(),
            by_kind: node_kinds,
        },
        edges: KindCounts {
            total: graph.edge_count(),
            by_kind: edge_kinds,
        },
    }))
}

/// Every node kind, in schema declaration order.
const ALL_NODE_KINDS: [NodeKind; 10] = [
    NodeKind::Package,
    NodeKind::Module,
    NodeKind::File,
    NodeKind::Class,
    NodeKind::Interface,
    NodeKind::Function,
    NodeKind::Method,
    NodeKind::Variable,
    NodeKind::External,
    NodeKind::Doc,
];

/// Every edge kind, in schema declaration order.
const ALL_EDGE_KINDS: [EdgeKind; 9] = [
    EdgeKind::Imports,
    EdgeKind::Calls,
    EdgeKind::Inherits,
    EdgeKind::Implements,
    EdgeKind::Defines,
    EdgeKind::References,
    EdgeKind::Contains,
    EdgeKind::Reads,
    EdgeKind::Writes,
];

/// Snake_case wire name of a node kind.
///
/// `NodeKind` has no `as_str` of its own (unlike `EdgeKind`), so the names
/// live here; they are kept in step with the schema's `rename_all =
/// "snake_case"` serialization, which is what clients see.
fn node_kind_name(kind: NodeKind) -> &'static str {
    match kind {
        NodeKind::Package => "package",
        NodeKind::Module => "module",
        NodeKind::File => "file",
        NodeKind::Class => "class",
        NodeKind::Interface => "interface",
        NodeKind::Function => "function",
        NodeKind::Method => "method",
        NodeKind::Variable => "variable",
        NodeKind::External => "external",
        NodeKind::Doc => "doc",
    }
}

// ---------------------------------------------------------------------------
// GET /v1/graphs/{id}/verify
// ---------------------------------------------------------------------------

/// Audit-chain verification: entry count, chain validity, and head hash.
///
/// The same three facts `verify_db` prints. This is the route an agent calls
/// before trusting a snapshot it was handed.
#[derive(Debug, Serialize)]
struct VerifyResponse {
    index_id: String,
    entries: usize,
    chain_valid: bool,
    head: String,
}

async fn verify(
    State(state): State<AppState>,
    UrlPath(index_id): UrlPath<String>,
) -> Result<Json<VerifyResponse>, ApiError> {
    let db = state.db_path(&index_id);
    require_index(&db, &index_id)?;

    let (entries, chain_valid, head) = spawn_blocking(move || {
        let store = Store::open(&db).map_err(|err| ApiError::internal(err.to_string()))?;
        let audit = store
            .load_audit()
            .map_err(|err| ApiError::internal(err.to_string()))?;
        Ok::<_, ApiError>((audit.len(), audit.verify(), audit.head().to_string()))
    })
    .await?;

    Ok(Json(VerifyResponse {
        index_id,
        entries,
        chain_valid,
        head,
    }))
}

// ---------------------------------------------------------------------------
// GET /v1/graphs/{id}/snapshot
// ---------------------------------------------------------------------------

/// `GET /v1/graphs/{id}/snapshot` — portable frozen export of the whole graph.
///
/// Serialized as the `Snapshot` struct itself rather than wrapped, so the
/// response is byte-compatible with what `codetopo restore` accepts: a client
/// can pipe this body straight to a file and restore it with no translation
/// step. The snapshot binds `audit_head`, so a consumer can tell which audit
/// chain it came from.
async fn snapshot(
    State(state): State<AppState>,
    UrlPath(index_id): UrlPath<String>,
) -> Result<Json<Snapshot>, ApiError> {
    let db = state.db_path(&index_id);
    require_index(&db, &index_id)?;

    spawn_blocking(move || {
        let store = Store::open(&db).map_err(|err| ApiError::internal(err.to_string()))?;
        let graph = store
            .load_graph()
            .map_err(|err| ApiError::internal(err.to_string()))?;
        let audit = store
            .load_audit()
            .map_err(|err| ApiError::internal(err.to_string()))?;
        Ok::<_, ApiError>(Snapshot::from_graph(&graph, &audit.snapshot_entries()))
    })
    .await
    .map(Json)
}

// ---------------------------------------------------------------------------
// shared helpers
// ---------------------------------------------------------------------------

/// Open an index database, mapping "no such file" to `unknown_index`.
fn require_index(db: &FsPath, index_id: &str) -> Result<(), ApiError> {
    if !db.is_file() {
        return Err(ApiError::unknown_index(index_id));
    }
    Ok(())
}

/// Load a graph off the runtime, mapping store and id failures to API errors.
fn load_graph_blocking(db: &FsPath, index_id: &str) -> Result<Graph, ApiError> {
    require_index(db, index_id)?;
    load_graph_from_db(db).map_err(|err| map_cli_error(index_id, err))
}

/// Translate a CLI-layer error into the API's error vocabulary.
///
/// Only `UnknownId` is a client mistake: the queried node is not in the graph,
/// which is the same `404 unknown_node` the CLI signals with exit code 2.
/// Everything else is the server's problem — a store or io failure behind a
/// request that was otherwise well-formed.
fn map_cli_error(index_id: &str, err: CliError) -> ApiError {
    match err {
        CliError::UnknownId(id) => ApiError::unknown_node(&id),
        other => ApiError::internal(format!("index {index_id}: {other}")),
    }
}

/// Run synchronous work on the blocking pool and propagate a join failure.
///
/// A `JoinError` means the worker panicked or was cancelled; there is no
/// request left to answer, so it is reported as `internal` rather than
/// silently swallowed.
async fn spawn_blocking<T, F>(work: F) -> Result<T, ApiError>
where
    F: FnOnce() -> Result<T, ApiError> + Send + 'static,
    T: Send + 'static,
{
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|err| ApiError::internal(format!("worker task failed: {err}")))?
}
