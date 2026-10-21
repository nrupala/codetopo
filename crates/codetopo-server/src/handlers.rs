// Copyright (C) 2026 Nrupal Akolkar
// SPDX-License-Identifier: AGPL-3.0-or-later
// Part of codetopo — Owned by Nrupal Akolkar (https://nrupalakolkar.com) · Built with Muse by Meta (https://muse.ai/join) — join Muse, then redeem code 101EEK in Settings within 48 hours of joining and we both earn tokens.

//! The request handlers of the HTTP API (Phase 2 design §3, extended by
//! the analytics-depth program: the symbol dossier, the diagnostics read,
//! the ranking, and the diff).
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

use axum::extract::{Extension, Path as UrlPath, Query, Request, State};
use axum::http::header::CONTENT_TYPE;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use codetopo_cli::{
    default_package_name, describe_schema, index_repo, load_graph_from_db, render_path,
    require_node, CliError,
};
use codetopo_core::graph::{Graph, PathStep};
use codetopo_core::schema::{EdgeKind, NodeKind};
use codetopo_core::snapshot::Snapshot;
use codetopo_store::Store;

use crate::auth::{KeyId, KeyPlan};
use crate::config::Config;
use crate::error::ApiError;
use crate::ingest;
use crate::keys::now_unix_secs;
use crate::limits::{plan_limits, Envelope};
use crate::tenancy::Tenancy;

/// Default `limit` for closure queries, matching the CLI's `--limit` default.
const DEFAULT_LIMIT: usize = 20;

/// Upper bound on `limit` (§3.1). A closure over a whole repository can hold
/// hundreds of thousands of ids, and an unbounded `limit` would let one request
/// allocate the whole graph's id set — the id list is the response, so the cap
/// is the only thing between a query and an out-of-memory.
const MAX_LIMIT: usize = 1000;

/// Shared, immutable server state.
///
/// The resolved configuration plus the tenancy store: the metering log
/// is owned by the auth layer, which is applied around the router
/// rather than inside it, and the key store is shared with that layer.
/// A handler still cannot reach a secret — [`KeyStore`] exposes none;
/// the state holds it for exactly one number, the active-key count the
/// ownerless-index rule turns on (see [`crate::tenancy`]). Tenancy is
/// different — a handler *must* consult it before touching an index —
/// and it carries no secrets, only ownership records. The [`Envelope`]
/// is the abuse-envelope state of paid-path S3: the query routes'
/// concurrency cap and per-key index-operation permits
/// (see [`crate::limits`]).
#[derive(Debug, Clone)]
pub struct AppState {
    config: Arc<Config>,
    keys: Arc<crate::keys::KeyStore>,
    tenancy: Arc<Tenancy>,
    envelope: Arc<Envelope>,
}

impl AppState {
    /// Wraps a resolved [`Config`] and the [`Tenancy`] store as axum state.
    ///
    /// Takes the `Arc`s the process already holds rather than values, so
    /// the state and the configuration `main` logs at startup are provably
    /// the same values and building a router in a test does not require
    /// cloning either. The envelope is process state, not configuration,
    /// so it is created here rather than passed in.
    pub fn new(
        config: Arc<Config>,
        keys: Arc<crate::keys::KeyStore>,
        tenancy: Arc<Tenancy>,
    ) -> Self {
        AppState {
            config,
            keys,
            tenancy,
            envelope: Arc::new(Envelope::new()),
        }
    }

    /// Database path for one index id (§6: `<data_dir>/<id>.db`).
    fn db_path(&self, index_id: &str) -> PathBuf {
        self.config.index_db_path(index_id)
    }
}

/// The tenancy gate every `/v1/graphs/:id/…` handler passes before it
/// touches the index (paid-path plan S1, gap G2).
///
/// * No ownership record: a legacy index (written before tenancy, or
///   placed by another tool) — readable only as
///   [`crate::tenancy::ownerless_readable`] rules: the operator's flag
///   when set, else only while the key store holds a single active
///   key. A denied legacy index is answered exactly like a missing
///   one, so the refusal cannot confirm the index exists either.
/// * Record owned by another key: answered exactly like a missing index
///   (`404 unknown_index`), so the response cannot confirm the index
///   exists. There is no `403` anywhere in this API's error vocabulary,
///   and a distinguishable "exists but not yours" would leak it.
/// * Expired record: refused the same way, and cleaned up on the spot —
///   the record is dropped and the database file deleted — so expiry is
///   enforced even between the startup sweeps in `Config::prepare`.
fn enforce_tenancy(state: &AppState, index_id: &str, key: &KeyId) -> Result<(), ApiError> {
    let Some(record) = state.tenancy.lookup(index_id)? else {
        if crate::tenancy::ownerless_readable(
            state.config.deny_ownerless_indexes,
            state.keys.active_len(),
        ) {
            return Ok(());
        }
        return Err(ApiError::unknown_index(index_id));
    };
    if record.is_expired(now_unix_secs()) {
        state.tenancy.remove(index_id)?;
        // The database and, for a hosted ingest, its staged tree:
        // an expired index leaves nothing behind.
        state.config.remove_index_files(index_id);
        return Err(ApiError::unknown_index(index_id));
    }
    if record.owner_key_id != key.as_str() {
        return Err(ApiError::unknown_index(index_id));
    }
    Ok(())
}

/// Build the unauthenticated route table.
///
/// [`crate::auth::metered`] wraps the result; this function only decides which
/// paths exist, so that auth coverage is a property of the wrapper rather than
/// something each route has to opt into. The index handler also takes the
/// [`KeyId`] and [`KeyPlan`] request extensions, which only that wrapper
/// inserts — serving this router without it answers `500` on that route
/// rather than skipping the tenancy and plan checks.
pub fn router(state: AppState) -> Router {
    // The query routes sit under the envelope middleware (global
    // concurrency cap + the caller's plan timeout, paid-path S3);
    // index creation does not — it has its own per-key permit and
    // ingest bounds inside the handler, and a creation's blocking
    // work must never be abandoned by a timer (see `index`).
    let envelope = state.envelope.clone();
    let queries = Router::new()
        .route("/v1/graphs/:id/descendants", get(descendants))
        .route("/v1/graphs/:id/ancestors", get(ancestors))
        .route("/v1/graphs/:id/blast-radius", get(blast_radius))
        .route("/v1/graphs/:id/path", get(path))
        .route("/v1/graphs/:id/stats", get(stats))
        .route("/v1/graphs/:id/verify", get(verify))
        .route("/v1/graphs/:id/snapshot", get(snapshot))
        .route("/v1/graphs/:id/neighborhood", get(neighborhood))
        .route("/v1/graphs/:id/symbol", get(symbol))
        .route("/v1/graphs/:id/search", get(search))
        .route("/v1/graphs/:id/diagnostics", get(diagnostics))
        .route("/v1/schema", get(schema))
        .route_layer(axum::middleware::from_fn_with_state(
            envelope,
            crate::limits::query_envelope,
        ));
    Router::new()
        .route("/v1/index", post(index))
        .merge(queries)
        .with_state(state)
}

// ---------------------------------------------------------------------------
// POST /v1/index
// ---------------------------------------------------------------------------

/// Body of `POST /v1/index` in JSON mode.
///
/// Every field is optional at the type level so that a *missing* field is
/// reported in the API's own error vocabulary naming it, rather than as a
/// generic deserialization rejection. The handler reads and parses the
/// body itself instead of using an extractor, because the same route
/// also serves the archive-upload mode, whose body is not JSON at all —
/// the content type, not the route, picks the mode (see [`index`]).
#[derive(Debug, Default, Deserialize)]
struct IndexRequest {
    repo_path: Option<String>,
    repo_url: Option<String>,
    package: Option<String>,
}

/// Query string of `POST /v1/index`. Only the archive mode reads it: an
/// upload has no JSON body to carry a `package`, so the name rides the
/// query string there. JSON mode takes `package` from the body.
#[derive(Debug, Default, Deserialize)]
struct IndexQuery {
    package: Option<String>,
}

// Package name recorded for the index when the request omits one:
// the shared default every surface uses
// (`codetopo_cli::default_package_name` — the repository directory's
// file stem). Before this was unified, the same fixture indexed as
// `fixture-repo::…` via CLI/MCP but `unknown::…` here, so node ids —
// and every query keyed on them — disagreed by surface.

/// The largest JSON body `POST /v1/index` reads: the limit the JSON
/// extractor enforced when it read the body, kept so taking over the
/// read does not silently change what a client may send.
const JSON_BODY_LIMIT: usize = 2 * 1024 * 1024;

/// A field's value, trimmed, with empty treated as absent — the rule
/// every optional field of this route follows.
fn non_empty(field: Option<String>) -> Option<String> {
    field
        .map(|raw| raw.trim().to_string())
        .filter(|raw| !raw.is_empty())
}

/// Reads a request body up to `limit`, mapping any failure (including
/// passing the limit) to `invalid_body` with the caller's wording.
async fn read_body(request: Request, limit: usize, failure: &str) -> Result<axum::body::Bytes, ApiError> {
    axum::body::to_bytes(request.into_body(), limit)
        .await
        .map_err(|_| ApiError::invalid_body(failure))
}

/// `POST /v1/index` — index a repository into a new index id.
///
/// Walks the tree, writes `<data_dir>/<index_id>.db`, and returns the CLI's own
/// `IndexReport` fields, so API stats and CLI stats come from one computation.
/// The index is recorded as owned by the creating key (see
/// [`enforce_tenancy`]), with the configured TTL as its expiry if one is set.
/// If the ownership record cannot be written, the fresh database is deleted
/// again rather than left behind as an unowned index anyone could read.
///
/// ## Ingest modes (paid-path S2, gap G1)
///
/// The request names the tree in exactly one of three ways:
///
/// * **`repo_path`** (JSON) — a path on the server host, offered only
///   to `self-hosted` keys. Naming host paths is the operator's own
///   power; a hosted key asking for it is refused `invalid_repo_path`.
/// * **`repo_url`** (JSON) — a public HTTPS git URL, validated and
///   shallow-cloned into the index's workdir
///   (`<data_dir>/ingest/<index_id>/`) by [`crate::ingest`]. Private
///   repositories are out of v1 scope: no credentials are accepted in
///   the URL, and the server holds none.
/// * **Archive upload** — the request body itself is a `.zip` or
///   `.tar.gz` (declared by content type, or sniffed from the magic
///   bytes), extracted into the same workdir under the containment
///   rules in [`crate::ingest`]. An explicit `package` rides the query
///   string (`?package=`), there being no JSON body to carry it.
///
/// `repo_path` and `repo_url` in one body is `invalid_body`; neither is
/// `invalid_repo_path`. An ingested tree belongs to its index: expiry
/// deletes the workdir with the database, and a failed ingest or index
/// removes the workdir before the error is answered — a partial tree is
/// never left behind.
///
/// ## Limits (paid-path S3, gap G4)
///
/// The caller's plan ([`crate::limits`]) bounds this route four ways,
/// all refused `429` before any ingest work is spent on them: one
/// index operation at a time per key (the permit is held until the
/// ownership record is written, so the next check is exact), an
/// active-index quota counted from the tenancy store, the plan's
/// ingest caps on the clone/upload itself, and a per-index file cap
/// counted over the resolved tree before indexing starts — a tree
/// past the cap leaves nothing behind, like any other failed ingest.
async fn index(
    State(state): State<AppState>,
    Extension(key): Extension<KeyId>,
    Extension(plan): Extension<KeyPlan>,
    Query(query): Query<IndexQuery>,
    request: Request,
) -> Result<Json<Value>, ApiError> {
    let content_type = request
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(|value| {
            value
                .split(';')
                .next()
                .unwrap_or("")
                .trim()
                .to_ascii_lowercase()
        })
        .filter(|value| !value.is_empty());

    // §6: a UUID v4 rather than a repo name, so re-indexing the same tree does
    // not clobber the previous index and no customer directory layout leaks
    // into a URL or a log line.
    let index_id = uuid::Uuid::new_v4().to_string();
    let db = state.db_path(&index_id);

    // The plan envelope (S3): this key's limits, the per-key index
    // permit, and its active-index quota — all before mode resolution,
    // so a refused creation costs no clone, no upload read, no walk.
    let limits = plan_limits(plan.as_str());
    let _index_permit = match limits.max_concurrent_index_ops {
        Some(max_ops) => match state.envelope.try_acquire_index_op(key.as_str(), max_ops) {
            Some(permit) => Some(permit),
            None => return Err(ApiError::index_concurrency(plan.as_str(), max_ops)),
        },
        None => None,
    };
    if let Some(max_active) = limits.max_active_indexes {
        let current = state
            .tenancy
            .count_active_for_owner(key.as_str(), now_unix_secs())?;
        if current >= max_active {
            return Err(ApiError::index_quota(plan.as_str(), max_active, current));
        }
    }
    let caps = limits.ingest;

    // Resolve the three modes to the tree to walk, the package name to
    // record, and the workdir this request owns (hosted modes only —
    // a local path is the caller's own tree and is never deleted here).
    let (repo, package, workdir) = if content_type.as_deref() == Some("application/json") {
        let bytes = read_body(
            request,
            JSON_BODY_LIMIT,
            "request body could not be read (2 MiB limit)",
        )
        .await?;
        let body: IndexRequest = serde_json::from_slice(&bytes).map_err(|err| {
            ApiError::invalid_body(&format!("not valid JSON for this route: {err}"))
        })?;
        match (non_empty(body.repo_path), non_empty(body.repo_url)) {
            (Some(_), Some(_)) => {
                return Err(ApiError::invalid_body(
                    "repo_path and repo_url are alternatives; send exactly one",
                ));
            }
            (None, None) => {
                return Err(ApiError::invalid_repo_path(
                    "one of repo_path or repo_url is required",
                ));
            }
            (Some(raw_path), None) => {
                if !plan.is_self_hosted() {
                    return Err(ApiError::invalid_repo_path(
                        "repo_path names a path on the server host and is available \
                         only to self-hosted keys; use repo_url or an archive upload",
                    ));
                }
                let repo = PathBuf::from(raw_path);
                if !repo.is_dir() {
                    return Err(ApiError::invalid_repo_path(&format!(
                        "not a directory: {}",
                        repo.display()
                    )));
                }
                let package =
                    non_empty(body.package).unwrap_or_else(|| default_package_name(&repo));
                (repo, package, None)
            }
            (None, Some(raw_url)) => {
                let validated = ingest::validate_repo_url(&raw_url).await?;
                let workdir = state.config.index_workdir_path(&index_id);
                let clone_dir = workdir.clone();
                let clone_validated = validated.clone();
                tokio::task::spawn_blocking(move || {
                    ingest::clone_into(&clone_validated, &clone_dir, caps)
                })
                .await
                .map_err(|err| ApiError::internal(format!("clone task failed: {err}")))??;
                let package =
                    non_empty(body.package).unwrap_or_else(|| validated.package_name());
                (workdir.clone(), package, Some(workdir))
            }
        }
    } else {
        let bytes = read_body(
            request,
            caps.max_upload_bytes,
            "archive body could not be read or exceeds the upload limit",
        )
        .await?;
        let kind = ingest::archive_kind(content_type.as_deref(), &bytes).ok_or_else(|| {
            ApiError::invalid_body(
                "unrecognized archive body: send a .zip or .tar.gz archive",
            )
        })?;
        let workdir = state.config.index_workdir_path(&index_id);
        let extract_dir = workdir.clone();
        let root = tokio::task::spawn_blocking(move || {
            ingest::extract_into(kind, &bytes, &extract_dir, caps)
        })
        .await
        .map_err(|err| ApiError::internal(format!("extract task failed: {err}")))??;
        let package = non_empty(query.package).unwrap_or_else(|| {
            if root != workdir {
                default_package_name(&root)
            } else {
                "repo".to_string()
            }
        });
        (root, package, Some(workdir))
    };

    // The per-index file cap (S3): counted over the resolved tree
    // before indexing starts, with the walk stopping just past the
    // cap. Counting first is what makes the refusal clean — no
    // extraction is paid for and no partial index exists to clean
    // up; the workdir removal is the same discipline every failed
    // ingest follows.
    if let Some(max_files) = limits.max_files_per_index {
        let count_root = repo.clone();
        let counted = tokio::task::spawn_blocking(move || {
            ingest::count_tree_files(&count_root, max_files as u64 + 1)
        })
        .await
        .map_err(|err| ApiError::internal(format!("file count task failed: {err}")))?;
        if counted > max_files as u64 {
            if let Some(workdir) = &workdir {
                let _ = std::fs::remove_dir_all(workdir);
            }
            return Err(ApiError::file_quota(plan.as_str(), max_files));
        }
    }

    let report = tokio::task::spawn_blocking(move || index_repo(&repo, &db, &package, None, None))
        .await
        .map_err(|err| ApiError::internal(format!("index task failed: {err}")))?;
    let report = match report {
        Ok(report) => report,
        Err(err) => {
            // A failed index leaves nothing behind: not the workdir,
            // and not a partially written database — an unrecorded
            // database file would read as a legacy index to any key.
            let _ = std::fs::remove_file(state.db_path(&index_id));
            if let Some(workdir) = &workdir {
                let _ = std::fs::remove_dir_all(workdir);
            }
            return Err(ApiError::internal(format!("cannot index repository: {err}")));
        }
    };

    let created_at = now_unix_secs();
    let expires_at = state
        .config
        .index_ttl_secs
        .map(|ttl| created_at + ttl as i64);
    if let Err(err) = state
        .tenancy
        .record(&index_id, key.as_str(), created_at, expires_at)
    {
        let _ = std::fs::remove_file(state.db_path(&index_id));
        if let Some(workdir) = &workdir {
            let _ = std::fs::remove_dir_all(workdir);
        }
        return Err(err);
    }

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
    Extension(key): Extension<KeyId>,
    UrlPath(index_id): UrlPath<String>,
    Query(query): Query<ClosureQuery>,
) -> Result<Json<ClosureResponse>, ApiError> {
    closure(state, &key, index_id, query, "descendants", Direction::Downstream).await
}

/// `GET /v1/graphs/{id}/ancestors?node=&expand=&limit=`.
///
/// The reverse closure: everything that transitively reaches this node.
async fn ancestors(
    State(state): State<AppState>,
    Extension(key): Extension<KeyId>,
    UrlPath(index_id): UrlPath<String>,
    Query(query): Query<ClosureQuery>,
) -> Result<Json<ClosureResponse>, ApiError> {
    closure(state, &key, index_id, query, "ancestors", Direction::Upstream).await
}

/// `GET /v1/graphs/{id}/blast-radius?node=&expand=&limit=`.
///
/// Breakage propagation: the dependency-side closure a failure of this node
/// would take with it.
async fn blast_radius(
    State(state): State<AppState>,
    Extension(key): Extension<KeyId>,
    UrlPath(index_id): UrlPath<String>,
    Query(query): Query<ClosureQuery>,
) -> Result<Json<ClosureResponse>, ApiError> {
    closure(
        state,
        &key,
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
    key: &KeyId,
    index_id: String,
    query: ClosureQuery,
    relation: &'static str,
    direction: Direction,
) -> Result<Json<ClosureResponse>, ApiError> {
    enforce_tenancy(&state, &index_id, key)?;
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
    Extension(key): Extension<KeyId>,
    UrlPath(index_id): UrlPath<String>,
    Query(query): Query<PathQuery>,
) -> Result<Json<PathResponse>, ApiError> {
    enforce_tenancy(&state, &index_id, &key)?;
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
    Extension(key): Extension<KeyId>,
    UrlPath(index_id): UrlPath<String>,
) -> Result<Json<StatsResponse>, ApiError> {
    enforce_tenancy(&state, &index_id, &key)?;
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
    Extension(key): Extension<KeyId>,
    UrlPath(index_id): UrlPath<String>,
) -> Result<Json<VerifyResponse>, ApiError> {
    enforce_tenancy(&state, &index_id, &key)?;
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
    Extension(key): Extension<KeyId>,
    UrlPath(index_id): UrlPath<String>,
) -> Result<Json<Snapshot>, ApiError> {
    enforce_tenancy(&state, &index_id, &key)?;
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

// ---------------------------------------------------------------------------
// GET /v1/graphs/{id}/neighborhood, /symbol, /search, /diagnostics;
// GET /v1/schema
// ---------------------------------------------------------------------------

/// Query for `neighborhood`: the shared closure params plus `depth`.
#[derive(Debug, Default, Deserialize)]
struct NeighborhoodQuery {
    node: Option<String>,
    depth: Option<String>,
}

/// `GET /v1/graphs/{id}/neighborhood?node=&depth=` — the subgraph
/// within `depth` hops of a node, both directions (CODE_SCHEMA §6's
/// neighborhood query; the CLI and MCP serve the same computation).
async fn neighborhood(
    State(state): State<AppState>,
    Extension(key): Extension<KeyId>,
    UrlPath(index_id): UrlPath<String>,
    Query(query): Query<NeighborhoodQuery>,
) -> Result<Json<Value>, ApiError> {
    enforce_tenancy(&state, &index_id, &key)?;
    let node = query
        .node
        .as_deref()
        .map(str::trim)
        .filter(|raw| !raw.is_empty())
        .ok_or_else(|| ApiError::missing_param("node"))?
        .to_string();
    let depth: u32 = match query.depth.as_deref().map(str::trim) {
        None | Some("") => 1,
        Some(raw) => raw.parse().map_err(|_| {
            ApiError::invalid_param("depth", &format!("expected a non-negative integer, got '{raw}'"))
        })?,
    };
    let db = state.db_path(&index_id);
    let graph = load_graph_blocking(&db, &index_id)?;
    require_node(&graph, &node).map_err(|e| map_cli_error(&index_id, e))?;
    let sub = graph.neighborhood(&node, depth);
    Ok(Json(json!({
        "index_id": index_id,
        "node": node,
        "depth": depth,
        "nodes": sub.nodes,
        "edges": sub.edges,
    })))
}

/// `GET /v1/graphs/{id}/symbol?node=` — one node in full: the DESIGN §3
/// dossier (the node, its incident edges grouped by kind and direction,
/// and the counts), serialized by the core type itself so the CLI and
/// MCP answers are the same object plus `index_id`.
async fn symbol(
    State(state): State<AppState>,
    Extension(key): Extension<KeyId>,
    UrlPath(index_id): UrlPath<String>,
    Query(query): Query<ClosureQuery>,
) -> Result<Json<Value>, ApiError> {
    enforce_tenancy(&state, &index_id, &key)?;
    let node_id = query
        .node
        .as_deref()
        .map(str::trim)
        .filter(|raw| !raw.is_empty())
        .ok_or_else(|| ApiError::missing_param("node"))?
        .to_string();
    let db = state.db_path(&index_id);
    let graph = load_graph_blocking(&db, &index_id)?;
    let dossier = graph
        .dossier(&node_id)
        .ok_or_else(|| ApiError::unknown_node(&node_id))?;
    let mut value = serde_json::to_value(&dossier)
        .map_err(|err| ApiError::internal(format!("dossier does not serialize: {err}")))?;
    value["index_id"] = json!(index_id);
    Ok(Json(value))
}

/// `GET /v1/graphs/{id}/diagnostics` — the diagnostics recorded when the
/// index was built (cycles, multi-definitions, unresolved calls, …),
/// read from the store exactly as the CLI prints them: the shared
/// [`codetopo_cli::diagnostics_value`] shape plus `index_id`.
async fn diagnostics(
    State(state): State<AppState>,
    Extension(key): Extension<KeyId>,
    UrlPath(index_id): UrlPath<String>,
) -> Result<Json<Value>, ApiError> {
    enforce_tenancy(&state, &index_id, &key)?;
    let db = state.db_path(&index_id);
    require_index(&db, &index_id)?;
    // The blocking closure is 'static, so it takes its own copy of the id
    // (the same pattern as the closure routes); the response still reports
    // the caller's `index_id`.
    let walk_index = index_id.clone();
    let mut value = spawn_blocking(move || {
        codetopo_cli::diagnostics_value(&db).map_err(|err| map_cli_error(&walk_index, err))
    })
    .await?;
    value["index_id"] = json!(index_id);
    Ok(Json(value))
}

/// Query for `search`.
#[derive(Debug, Default, Deserialize)]
struct SearchQuery {
    q: Option<String>,
    limit: Option<String>,
}

/// `GET /v1/graphs/{id}/search?q=&limit=` — name discovery: the step
/// the exact-id queries always assumed a caller had already done.
async fn search(
    State(state): State<AppState>,
    Extension(key): Extension<KeyId>,
    UrlPath(index_id): UrlPath<String>,
    Query(query): Query<SearchQuery>,
) -> Result<Json<Value>, ApiError> {
    enforce_tenancy(&state, &index_id, &key)?;
    let q = query
        .q
        .as_deref()
        .map(str::trim)
        .filter(|raw| !raw.is_empty())
        .ok_or_else(|| ApiError::missing_param("q"))?
        .to_string();
    let limit: usize = match query.limit.as_deref().map(str::trim) {
        None | Some("") => 20,
        Some(raw) => raw.parse().map_err(|_| {
            ApiError::invalid_param("limit", &format!("expected a positive integer, got '{raw}'"))
        })?,
    };
    let db = state.db_path(&index_id);
    let graph = load_graph_blocking(&db, &index_id)?;
    let hits = graph.search_symbols(&q, limit);
    Ok(Json(json!({
        "index_id": index_id,
        "query": q,
        "total": hits.len(),
        "results": hits,
    })))
}

/// `GET /v1/schema` — the schema description, identical to the CLI's
/// `schema` command and the MCP `codetopo_describe_schema` tool
/// (one generator in the CLI library serves all three).
async fn schema() -> Json<Value> {
    Json(describe_schema())
}

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
