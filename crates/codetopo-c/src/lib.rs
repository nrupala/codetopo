// Copyright (C) 2026 2351485 ALBERTA INC.
// SPDX-License-Identifier: AGPL-3.0-or-later
// Part of codetopo — Owned by 2351485 ALBERTA INC. (https://nrupalakolkar.com) · Built with Muse by Meta (https://muse.ai/join).

//! codetopo-c — the C ABI foundation for codetopo.
//!
//! A thin FFI crate over `codetopo-cli`'s library (which carries
//! indexing and the shared `*_value` query builders) and
//! `codetopo-core`'s frozen [`Graph`]. It exists as its own crate —
//! rather than as a crate-type change inside `codetopo-core` —
//! because indexing lives in `codetopo-cli`, which depends on the
//! core: an in-core FFI could not expose indexing without a
//! dependency cycle.
//!
//! The boundary is CODE_SCHEMA JSON strings (§7: JSON is the
//! universal pathway), so every answer is the same `serde_json::Value`
//! the CLI/MCP/HTTP surfaces produce — parity by construction. The
//! full contract (ownership, error codes, the operation list) is
//! documented in `include/codetopo.h`, which is the header of record
//! and is pinned against this crate's exports by tests.
//!
//! No panic unwinds across the boundary: every entry point runs
//! inside `catch_unwind` and converts a panic into an `internal`
//! error (or a no-op for the free functions).
//!
//! The pointer-taking entry points are declared `unsafe extern "C"`:
//! a Rust-side marker only (the caller must uphold the header's
//! pointer contract, as with any C API) — the C ABI, the exported
//! symbols, and the header are exactly what a C caller links against.

use std::ffi::{CStr, CString};
use std::os::raw::c_char;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use codetopo_cli::CliError;
use codetopo_core::{Graph, RankMetric, Snapshot};
use codetopo_store::{AuditLog, Store};
use serde_json::{json, Value};

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// One boundary error: a stable machine-readable `code` (the existing
/// surfaces' vocabulary) plus a human-readable `message`. Rendered to
/// the caller as the JSON object `{"code": ..., "message": ...}`.
struct CError {
    code: &'static str,
    message: String,
}

impl CError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        CError { code, message: message.into() }
    }

    fn invalid_param(message: impl Into<String>) -> Self {
        Self::new("invalid_param", message)
    }

    fn internal(message: impl Into<String>) -> Self {
        Self::new("internal", message)
    }

    fn to_json_cstring(&self) -> *mut c_char {
        let text = json!({"code": self.code, "message": self.message}).to_string();
        CString::new(text)
            .map(CString::into_raw)
            .unwrap_or(std::ptr::null_mut())
    }
}

/// Map the CLI library's errors onto the boundary vocabulary. The
/// CLI's `UnknownId` is the symbol-not-in-graph case every surface
/// reports (`unknown_symbol` here, after the task vocabulary; the
/// CLI's exit-code-2 case).
fn map_cli_error(err: CliError) -> CError {
    match err {
        CliError::UnknownId(id) => {
            CError::new("unknown_symbol", format!("unknown id '{id}': not present in the graph"))
        }
        CliError::Store(e) => CError::new("store_error", format!("store error: {e}")),
        CliError::Io(e) => CError::new("io_error", format!("io error: {e}")),
        CliError::Json(e) => CError::invalid_param(format!("json error: {e}")),
        CliError::Usage(msg) => {
            if msg.contains("unsupported snapshot schema_version") {
                CError::new("unsupported_schema_version", msg)
            } else {
                CError::invalid_param(msg)
            }
        }
    }
}

// ---------------------------------------------------------------------------
// FFI plumbing
// ---------------------------------------------------------------------------

/// Borrow a required C string argument as UTF-8.
unsafe fn cstr_required<'a>(ptr: *const c_char, name: &str) -> Result<&'a str, CError> {
    if ptr.is_null() {
        return Err(CError::invalid_param(format!("{name} must not be NULL")));
    }
    unsafe { CStr::from_ptr(ptr) }
        .to_str()
        .map_err(|_| CError::invalid_param(format!("{name} is not valid UTF-8")))
}

/// Borrow an optional C string argument (`NULL` = absent).
unsafe fn cstr_optional<'a>(ptr: *const c_char, name: &str) -> Result<Option<&'a str>, CError> {
    if ptr.is_null() {
        return Ok(None);
    }
    unsafe { cstr_required(ptr, name) }.map(Some)
}

/// Write an error JSON string into `err_out` (when non-NULL). The
/// caller owns the stored string and frees it with
/// [`codetopo_free_string`].
fn set_err(err_out: *mut *mut c_char, err: &CError) {
    if !err_out.is_null() {
        unsafe { *err_out = err.to_json_cstring() };
    }
}

/// Clear `err_out` (when non-NULL) — the success half of the contract.
fn clear_err(err_out: *mut *mut c_char) {
    if !err_out.is_null() {
        unsafe { *err_out = std::ptr::null_mut() };
    }
}

/// Serialize a query answer and hand ownership of its C string to the
/// caller.
fn value_to_c(value: Value) -> Result<*mut c_char, CError> {
    let text = serde_json::to_string(&value)
        .map_err(|e| CError::internal(format!("answer does not serialize: {e}")))?;
    CString::new(text)
        .map(CString::into_raw)
        .map_err(|_| CError::internal("answer contains an interior NUL byte"))
}

/// Run a fallible pointer-returning body, converting errors and
/// panics alike into `NULL` + `err_out`.
fn guard<T>(
    err_out: *mut *mut c_char,
    body: impl FnOnce() -> Result<T, CError>,
) -> Result<T, ()> {
    match catch_unwind(AssertUnwindSafe(body)) {
        Ok(Ok(value)) => {
            clear_err(err_out);
            Ok(value)
        }
        Ok(Err(err)) => {
            set_err(err_out, &err);
            Err(())
        }
        Err(_) => {
            set_err(
                err_out,
                &CError::internal("internal panic caught at the C boundary"),
            );
            Err(())
        }
    }
}

// ---------------------------------------------------------------------------
// The graph handle
// ---------------------------------------------------------------------------

/// Where a handle's graph came from — it decides how the three
/// store-shaped operations (`diagnostics`, `verify`, `snapshot`)
/// answer, exactly as the other surfaces decide it.
enum HandleSource {
    /// Opened from an index database: diagnostics and the audit chain
    /// are read back from the store at this path.
    Db(PathBuf),
    /// Loaded from a Snapshot: the snapshot itself is retained, so
    /// `verify` checks the audit log it carries and `snapshot`
    /// returns its own content.
    Snapshot(Box<Snapshot>),
}

/// The opaque handle behind `codetopo_graph`. Owns the frozen graph;
/// queries borrow it and never mutate it. The type name is the C
/// type name — the ABI contract fixes it, hence the allow.
#[allow(non_camel_case_types)]
pub struct codetopo_graph {
    graph: Graph,
    source: HandleSource,
}

// ---------------------------------------------------------------------------
// Version functions
// ---------------------------------------------------------------------------

fn static_cstring(text: &str) -> *const c_char {
    static CACHE: OnceLock<Vec<CString>> = OnceLock::new();
    // Two static strings live here for the life of the process.
    let entries = CACHE.get_or_init(|| {
        vec![
            CString::new(env!("CODETOPO_CORE_VERSION")).expect("version is a valid C string"),
            CString::new(codetopo_core::snapshot::SCHEMA_VERSION)
                .expect("schema version is a valid C string"),
        ]
    });
    entries[if text == "schema" { 1 } else { 0 }].as_ptr()
}

/// Version of the codetopo core this library was built against.
/// Static string — the caller must NOT free it. See the header.
#[no_mangle]
pub extern "C" fn codetopo_version() -> *const c_char {
    static_cstring("version")
}

/// CODE_SCHEMA version this build speaks, from the core's
/// `SCHEMA_VERSION` constant. Static string — do NOT free.
#[no_mangle]
pub extern "C" fn codetopo_schema_version() -> *const c_char {
    static_cstring("schema")
}

// ---------------------------------------------------------------------------
// Handle lifecycle + indexing
// ---------------------------------------------------------------------------

/// Open the index database at `db_path`. See the header for the
/// ownership and error contract.
///
/// # Safety
/// All pointer arguments must follow the contract in
/// `include/codetopo.h`: string arguments are valid, NUL-terminated
/// C strings (or NULL where the header allows it), and `err_out` is
/// either NULL or a valid writable `char *` slot.
#[no_mangle]
pub unsafe extern "C" fn codetopo_open_db(
    db_path: *const c_char,
    err_out: *mut *mut c_char,
) -> *mut codetopo_graph {
    let body = || -> Result<*mut codetopo_graph, CError> {
        let db_path = unsafe { cstr_required(db_path, "db_path") }?;
        let path = PathBuf::from(db_path);
        // Store::open creates a missing database, so existence is
        // asserted up front — the MCP surface's require_index rule:
        // a missing index must not look like an empty repository.
        if !path.is_file() {
            return Err(CError::new(
                "unknown_index",
                format!("unknown index: no database at '{db_path}'"),
            ));
        }
        let graph = codetopo_cli::load_graph_from_db(&path).map_err(map_cli_error)?;
        Ok(Box::into_raw(Box::new(codetopo_graph {
            graph,
            source: HandleSource::Db(path),
        })))
    };
    guard(err_out, body).unwrap_or(std::ptr::null_mut())
}

/// Load a graph from a Snapshot JSON string. See the header.
///
/// # Safety
/// All pointer arguments must follow the contract in
/// `include/codetopo.h`: string arguments are valid, NUL-terminated
/// C strings (or NULL where the header allows it), and `err_out` is
/// either NULL or a valid writable `char *` slot.
#[no_mangle]
pub unsafe extern "C" fn codetopo_load_snapshot(
    snapshot_json: *const c_char,
    err_out: *mut *mut c_char,
) -> *mut codetopo_graph {
    let body = || -> Result<*mut codetopo_graph, CError> {
        let text = unsafe { cstr_required(snapshot_json, "snapshot_json") }?;
        let snap = Snapshot::from_json(text).map_err(|e| {
            CError::invalid_param(format!("snapshot_json is not a valid snapshot: {e}"))
        })?;
        // CODE_SCHEMA §8: reject a major schema mismatch loudly.
        snap.check_schema_version()
            .map_err(|msg| CError::new("unsupported_schema_version", msg))?;
        let graph = snap.to_graph();
        Ok(Box::into_raw(Box::new(codetopo_graph {
            graph,
            source: HandleSource::Snapshot(Box::new(snap)),
        })))
    };
    guard(err_out, body).unwrap_or(std::ptr::null_mut())
}

/// Release a graph handle. NULL is a no-op.
///
/// # Safety
/// `graph` must be NULL or a handle returned by
/// `codetopo_open_db` / `codetopo_load_snapshot` that has not
/// already been freed.
#[no_mangle]
pub unsafe extern "C" fn codetopo_graph_free(graph: *mut codetopo_graph) {
    if graph.is_null() {
        return;
    }
    let _ = catch_unwind(AssertUnwindSafe(|| unsafe {
        drop(Box::from_raw(graph));
    }));
}

/// Release a string returned by this library. NULL is a no-op. Must
/// not be called on the static version strings.
///
/// # Safety
/// `s` must be NULL or a string returned by this library that has
/// not already been freed.
#[no_mangle]
pub unsafe extern "C" fn codetopo_free_string(s: *mut c_char) {
    if s.is_null() {
        return;
    }
    let _ = catch_unwind(AssertUnwindSafe(|| unsafe {
        drop(CString::from_raw(s));
    }));
}

/// Index a repository into a database. See the header.
///
/// # Safety
/// All pointer arguments must follow the contract in
/// `include/codetopo.h`: string arguments are valid, NUL-terminated
/// C strings (or NULL where the header allows it), and `err_out` is
/// either NULL or a valid writable `char *` slot.
#[no_mangle]
pub unsafe extern "C" fn codetopo_index(
    repo_path: *const c_char,
    db_path: *const c_char,
    package: *const c_char,
    err_out: *mut *mut c_char,
) -> *mut c_char {
    let body = || -> Result<*mut c_char, CError> {
        let repo_path = unsafe { cstr_required(repo_path, "repo_path") }?;
        let db_path = unsafe { cstr_required(db_path, "db_path") }?;
        let package = unsafe { cstr_optional(package, "package") }?;
        let repo = Path::new(repo_path);
        let package = match package {
            Some(p) if !p.is_empty() => p.to_string(),
            _ => codetopo_cli::default_package_name(repo),
        };
        let report = codetopo_cli::index_repo(repo, Path::new(db_path), &package, None, None)
            .map_err(map_cli_error)?;
        value_to_c(json!({
            "package": package,
            "files_indexed": report.files_indexed,
            "files_failed": report.files_failed,
            "nodes": report.nodes,
            "edges": report.edges,
            "diagnostics": report.diagnostics,
        }))
    };
    guard(err_out, body).unwrap_or(std::ptr::null_mut())
}

// ---------------------------------------------------------------------------
// Query dispatch
// ---------------------------------------------------------------------------

/// Parse the `args_json` argument: NULL/empty = `{}`, and the value
/// must be a JSON object — anything else is `invalid_param`, never a
/// silent default.
fn parse_args(args_json: *const c_char) -> Result<Value, CError> {
    let text = unsafe { cstr_optional(args_json, "args_json") }?;
    let Some(text) = text.filter(|t| !t.trim().is_empty()) else {
        return Ok(json!({}));
    };
    let value: Value = serde_json::from_str(text)
        .map_err(|e| CError::invalid_param(format!("args_json is not valid JSON: {e}")))?;
    if !value.is_object() {
        return Err(CError::invalid_param("args_json must be a JSON object"));
    }
    Ok(value)
}

/// A required string argument (`node`, `query`, …), read under its
/// canonical key or an accepted alias. Absent/blank/non-string is
/// `invalid_param` naming the canonical key.
fn req_str(args: &Value, key: &str, aliases: &[&str]) -> Result<String, CError> {
    for k in std::iter::once(&key).chain(aliases.iter()) {
        if let Some(s) = args.get(*k).and_then(Value::as_str) {
            if !s.trim().is_empty() {
                return Ok(s.to_string());
            }
        }
    }
    Err(CError::invalid_param(format!("missing or invalid parameter: {key}")))
}

/// An optional string argument.
fn opt_str(args: &Value, key: &str) -> Result<Option<String>, CError> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) if !s.trim().is_empty() => Ok(Some(s.clone())),
        Some(_) => Err(CError::invalid_param(format!("parameter '{key}' must be a string"))),
    }
}

/// An optional unsigned-integer argument with a default.
fn opt_usize(args: &Value, key: &str, default: usize) -> Result<usize, CError> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(default),
        Some(Value::Number(n)) => n
            .as_u64()
            .map(|v| usize::try_from(v).unwrap_or(usize::MAX))
            .ok_or_else(|| {
                CError::invalid_param(format!("parameter '{key}' must be a non-negative integer"))
            }),
        Some(_) => Err(CError::invalid_param(format!(
            "parameter '{key}' must be a non-negative integer"
        ))),
    }
}

/// An optional boolean argument (default false).
fn opt_bool(args: &Value, key: &str) -> Result<bool, CError> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(false),
        Some(Value::Bool(b)) => Ok(*b),
        Some(_) => Err(CError::invalid_param(format!("parameter '{key}' must be a boolean"))),
    }
}

/// Fetch a node id or fail `unknown_symbol` — an empty closure for a
/// typo'd id would be indistinguishable from a leaf (the MCP
/// surface's rule, CODE_SCHEMA §3).
fn require_node_id(graph: &Graph, id: &str) -> Result<(), CError> {
    if graph.get(id).is_some() {
        Ok(())
    } else {
        Err(CError::new(
            "unknown_symbol",
            format!("unknown id '{id}': not present in the graph"),
        ))
    }
}

/// The stats answer, in the CLI/HTTP shape
/// `{nodes: {total, by_kind}, edges: {total, by_kind}}` (the shape
/// `codetopo_cli::print_stats_json` prints), tallied with the CLI's
/// own kind tables.
fn stats_value(graph: &Graph) -> Value {
    let mut node_kinds = serde_json::Map::new();
    for kind in codetopo_cli::all_node_kinds() {
        let count = graph.nodes().filter(|n| n.kind == kind).count();
        if count > 0 {
            node_kinds.insert(codetopo_cli::node_kind_name(kind).to_string(), count.into());
        }
    }
    let mut edge_kinds = serde_json::Map::new();
    for kind in codetopo_cli::all_edge_kinds() {
        let count = graph.edge_count_by_kind(kind);
        if count > 0 {
            edge_kinds.insert(kind.as_str().to_string(), count.into());
        }
    }
    json!({
        "nodes": {"total": graph.node_count(), "by_kind": node_kinds},
        "edges": {"total": graph.edge_count(), "by_kind": edge_kinds},
    })
}

/// The closure answer shared by descendants / ancestors /
/// blast_radius, field-for-field the MCP surface's shape: the total,
/// the truncation state, and the members (ids, or full node records
/// with `expand`).
fn closure_value(
    graph: &Graph,
    members: Vec<String>,
    expand: bool,
    limit: usize,
) -> Value {
    let total = members.len();
    let shown: Vec<String> = members.into_iter().take(limit).collect();
    if !expand {
        return json!({
            "total": total,
            "returned": shown.len(),
            "truncated": total > shown.len(),
            "limit": limit,
            "expand": false,
            "nodes": shown,
        });
    }
    let expanded: Vec<Value> = shown
        .iter()
        .filter_map(|id| graph.get(id))
        .filter_map(|node| serde_json::to_value(node).ok())
        .collect();
    json!({
        "total": total,
        "returned": expanded.len(),
        "truncated": total > expanded.len(),
        "limit": limit,
        "expand": true,
        "nodes": expanded,
    })
}

fn query_inner(handle: &codetopo_graph, op: &str, args: &Value) -> Result<Value, CError> {
    let graph = &handle.graph;
    match op {
        "describe_schema" => Ok(codetopo_cli::describe_schema()),
        "stats" => Ok(stats_value(graph)),
        "search_symbols" => {
            let query = req_str(args, "query", &["q"])?;
            let limit = opt_usize(args, "limit", 20)?;
            let hits = graph.search_symbols(&query, limit);
            Ok(json!({"query": query, "total": hits.len(), "results": hits}))
        }
        "get_symbol" => {
            let id = req_str(args, "node", &["id"])?;
            let dossier = graph
                .dossier(&id)
                .ok_or_else(|| CError::new("unknown_symbol", format!("unknown id '{id}': not present in the graph")))?;
            serde_json::to_value(dossier)
                .map_err(|e| CError::internal(format!("dossier does not serialize: {e}")))
        }
        "descendants" | "ancestors" | "blast_radius" => {
            let id = req_str(args, "node", &["id"])?;
            require_node_id(graph, &id)?;
            let expand = opt_bool(args, "expand")?;
            let limit = opt_usize(args, "limit", 20)?;
            let members = match op {
                "descendants" => graph.descendants(&id),
                "ancestors" => graph.ancestors(&id),
                _ => graph.blast_radius(&id),
            };
            Ok(closure_value(graph, members, expand, limit))
        }
        "path" => {
            let from = req_str(args, "from", &[])?;
            let to = req_str(args, "to", &[])?;
            require_node_id(graph, &from)?;
            require_node_id(graph, &to)?;
            let steps = graph.path(&from, &to).ok_or_else(|| {
                CError::new("no_path", format!("no path from {from} to {to}"))
            })?;
            // The MCP surface's shape: the typed steps, the two
            // endpoint records, and the edges the steps traverse.
            let nodes: Vec<Value> = [from.as_str(), to.as_str()]
                .into_iter()
                .filter_map(|id| graph.get(id))
                .filter_map(|node| serde_json::to_value(node).ok())
                .collect();
            let edges: Vec<Value> = steps
                .iter()
                .filter_map(|step| {
                    graph
                        .edges()
                        .find(|e| e.from == step.from && e.to == step.to && e.kind == step.kind)
                })
                .filter_map(|edge| serde_json::to_value(edge).ok())
                .collect();
            Ok(json!({"path": steps, "nodes": nodes, "edges": edges}))
        }
        "neighborhood" => {
            let id = req_str(args, "node", &["id"])?;
            require_node_id(graph, &id)?;
            let depth = u32::try_from(opt_usize(args, "depth", 1)?).unwrap_or(u32::MAX);
            let sub = graph.neighborhood(&id, depth);
            Ok(json!({
                "node": id,
                "depth": depth,
                "nodes": sub.nodes,
                "edges": sub.edges,
            }))
        }
        "diagnostics" => match &handle.source {
            HandleSource::Db(db_path) => {
                codetopo_cli::diagnostics_value(db_path).map_err(map_cli_error)
            }
            // Snapshots carry no diagnostics (they are recorded in
            // the store at index time), so the honest answer for a
            // snapshot-backed handle is the empty list — stated in
            // the header, not hidden.
            HandleSource::Snapshot(_) => Ok(json!({"total": 0, "diagnostics": []})),
        },
        "rank" => {
            // The 0.9.1 discipline at this boundary too: `by` is the
            // ONLY metric parameter name. Any other key — a plausible
            // `metric`, or anything unexpected — is an error naming
            // the key, never a silently-ignored parameter answered
            // with the default metric.
            if let Some(obj) = args.as_object() {
                for key in obj.keys() {
                    if key != "by" && key != "limit" {
                        return Err(CError::invalid_param(format!(
                            "unknown rank parameter '{key}': rank's metric parameter is 'by' only"
                        )));
                    }
                }
            }
            let by = opt_str(args, "by")?.unwrap_or_else(|| "blast_radius".to_string());
            let metric = RankMetric::parse(&by).ok_or_else(|| {
                CError::invalid_param(format!(
                    "unknown metric for 'by': '{by}' (expected one of fan_in, fan_out, \
                     blast_radius, complexity, pagerank, betweenness, closeness, \
                     eigenvector, katz)"
                ))
            })?;
            let limit = opt_usize(args, "limit", 20)?;
            Ok(codetopo_cli::rank_value(graph, metric, limit))
        }
        "metrics" => Ok(codetopo_cli::metrics_value(graph)),
        "communities" => {
            let limit = opt_usize(args, "limit", 20)?;
            Ok(codetopo_cli::communities_value(graph, limit))
        }
        "topo_sort" => Ok(codetopo_cli::topo_sort_value(graph)),
        "render" => {
            let layout = opt_str(args, "layout")?.unwrap_or_else(|| "layered".to_string());
            codetopo_cli::render_value(graph, &layout).map_err(CError::invalid_param)
        }
        "history" => {
            let min_support = opt_usize(args, "min_support", 2)?;
            let (input, reason) = match opt_str(args, "repo_path")? {
                Some(repo) => {
                    match codetopo_cli::repo_history_input(
                        graph,
                        Path::new(&repo),
                        opt_str(args, "since")?.as_deref(),
                    ) {
                        Ok(input) => (Some(input), String::new()),
                        Err(reason) => (None, reason),
                    }
                }
                None => (
                    None,
                    "no working tree supplied: history is measured from a local \
                     repository (pass repo_path); an index alone carries no history"
                        .to_string(),
                ),
            };
            Ok(codetopo_cli::history_value(graph, input, min_support, &reason))
        }
        "diff" => {
            let other = load_other_graph(args)?;
            Ok(codetopo_cli::diff_value(graph, &other))
        }
        "snapshot" => match &handle.source {
            HandleSource::Db(db_path) => {
                let store = Store::open(db_path)
                    .map_err(|e| CError::new("store_error", format!("store error: {e}")))?;
                let stored = store
                    .load_graph()
                    .map_err(|e| CError::new("store_error", format!("store error: {e}")))?;
                let audit = store
                    .load_audit()
                    .map_err(|e| CError::new("store_error", format!("store error: {e}")))?;
                let diagnostics = store
                    .load_diagnostics()
                    .map_err(|e| CError::new("store_error", format!("store error: {e}")))?;
                let snap = Snapshot::from_graph_with_diagnostics(
                    &stored,
                    &audit.snapshot_entries(),
                    &diagnostics,
                );
                serde_json::to_value(snap)
                    .map_err(|e| CError::internal(format!("snapshot does not serialize: {e}")))
            }
            HandleSource::Snapshot(snap) => serde_json::to_value(snap.as_ref().clone())
                .map_err(|e| CError::internal(format!("snapshot does not serialize: {e}"))),
        },
        "verify" => match &handle.source {
            HandleSource::Db(db_path) => {
                let store = Store::open(db_path)
                    .map_err(|e| CError::new("store_error", format!("store error: {e}")))?;
                let audit = store
                    .load_audit()
                    .map_err(|e| CError::new("store_error", format!("store error: {e}")))?;
                Ok(json!({
                    "entries": audit.len(),
                    "chain_valid": audit.verify(),
                    "head": audit.head(),
                }))
            }
            HandleSource::Snapshot(snap) => {
                let audit = AuditLog::from_snapshot_entries(&snap.audit_log);
                let chain_valid = audit.verify() && audit.head() == snap.audit_head;
                Ok(json!({
                    "entries": snap.audit_log.len(),
                    "chain_valid": chain_valid,
                    "head": snap.audit_head,
                }))
            }
        },
        other => Err(CError::invalid_param(format!(
            "unknown operation '{other}': expected one of describe_schema, stats, \
             search_symbols, get_symbol, descendants, ancestors, blast_radius, path, \
             neighborhood, diagnostics, rank, metrics, communities, topo_sort, render, \
             history, diff, snapshot, verify"
        ))),
    }
}

/// The `diff` op's other graph: exactly one source key, as on the
/// CLI/MCP surfaces (`other_*` names this handle as the "before"
/// side; the MCP `after_*` spellings are accepted for the same slot).
fn load_other_graph(args: &Value) -> Result<Graph, CError> {
    let db = opt_str(args, "other_db_path")?.or(opt_str(args, "after_db_path")?);
    let snap_json = opt_str(args, "other_snapshot_json")?.or(opt_str(args, "after_snapshot_json")?);
    let snap_path = opt_str(args, "other_snapshot_path")?
        .or(opt_str(args, "other_snapshot")?)
        .or(opt_str(args, "after_snapshot")?)
        .or(opt_str(args, "after_snapshot_path")?);
    match (db, snap_json, snap_path) {
        (Some(db), None, None) => {
            let path = PathBuf::from(&db);
            if !path.is_file() {
                return Err(CError::new(
                    "unknown_index",
                    format!("unknown index: no database at '{db}'"),
                ));
            }
            codetopo_cli::load_graph_from_db(&path).map_err(map_cli_error)
        }
        (None, Some(text), None) => {
            let snap = Snapshot::from_json(&text).map_err(|e| {
                CError::invalid_param(format!("other_snapshot_json is not a valid snapshot: {e}"))
            })?;
            snap.check_schema_version()
                .map_err(|msg| CError::new("unsupported_schema_version", msg))?;
            Ok(snap.to_graph())
        }
        (None, None, Some(path)) => {
            codetopo_cli::read_snapshot_graph(Path::new(&path)).map_err(map_cli_error)
        }
        _ => Err(CError::invalid_param(
            "diff requires exactly one other source: 'other_db_path', \
             'other_snapshot_json', or 'other_snapshot_path'",
        )),
    }
}

/// Run one query operation against a graph handle. See the header
/// for the operation list, argument shapes, and error contract.
///
/// # Safety
/// `graph` must be a live handle from `codetopo_open_db` /
/// `codetopo_load_snapshot`; string arguments must follow the
/// header's contract, and `err_out` must be NULL or a valid
/// writable `char *` slot.
#[no_mangle]
pub unsafe extern "C" fn codetopo_query(
    graph: *const codetopo_graph,
    op: *const c_char,
    args_json: *const c_char,
    err_out: *mut *mut c_char,
) -> *mut c_char {
    let body = || -> Result<*mut c_char, CError> {
        if graph.is_null() {
            return Err(CError::invalid_param("graph handle must not be NULL"));
        }
        let handle = unsafe { &*graph };
        let op = unsafe { cstr_required(op, "op") }?;
        let args = parse_args(args_json)?;
        let value = query_inner(handle, op, &args)?;
        value_to_c(value)
    };
    guard(err_out, body).unwrap_or(std::ptr::null_mut())
}

// ---------------------------------------------------------------------------
// Tests (in-process, through the extern fns themselves)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::CString;

    fn c(text: &str) -> CString {
        CString::new(text).expect("test string is a valid C string")
    }

    /// Take ownership of a returned string and read it as JSON.
    fn take_json(ptr: *mut c_char) -> Value {
        assert!(!ptr.is_null(), "expected a non-NULL answer");
        let text = unsafe { CString::from_raw(ptr) }
            .into_string()
            .expect("answer is UTF-8");
        serde_json::from_str(&text).expect("answer is JSON")
    }

    /// Take ownership of an error string and read it as JSON.
    fn take_err(ptr: *mut c_char) -> Value {
        assert!(!ptr.is_null(), "expected a non-NULL error string");
        let text = unsafe { CString::from_raw(ptr) }
            .into_string()
            .expect("error is UTF-8");
        serde_json::from_str(&text).expect("error is a JSON object")
    }

    // Safe test wrappers over the unsafe FFI entry points: each one
    // owns the err_out slot discipline (NULL on success, a JSON
    // error object on failure) so the tests read as assertions about
    // behaviour, not about pointer plumbing.

    fn call_index(repo: &Path, db: &Path, package: Option<&str>) -> Result<Value, Value> {
        let mut err: *mut c_char = std::ptr::null_mut();
        let pkg = package.map(c);
        let ptr = unsafe {
            codetopo_index(
                c(&repo.display().to_string()).as_ptr(),
                c(&db.display().to_string()).as_ptr(),
                pkg.as_ref().map_or(std::ptr::null(), |p| p.as_ptr()),
                &mut err,
            )
        };
        if ptr.is_null() {
            Err(take_err(err))
        } else {
            assert!(err.is_null(), "err_out set on success");
            Ok(take_json(ptr))
        }
    }

    fn call_open_db(db: &Path) -> Result<*mut codetopo_graph, Value> {
        let mut err: *mut c_char = std::ptr::null_mut();
        let handle = unsafe {
            codetopo_open_db(c(&db.display().to_string()).as_ptr(), &mut err)
        };
        if handle.is_null() {
            Err(take_err(err))
        } else {
            assert!(err.is_null(), "err_out set on success");
            Ok(handle)
        }
    }

    fn call_load_snapshot(text: &str) -> Result<*mut codetopo_graph, Value> {
        let mut err: *mut c_char = std::ptr::null_mut();
        let handle = unsafe { codetopo_load_snapshot(c(text).as_ptr(), &mut err) };
        if handle.is_null() {
            Err(take_err(err))
        } else {
            assert!(err.is_null(), "err_out set on success");
            Ok(handle)
        }
    }

    fn call_query(handle: *const codetopo_graph, op: &str, args: &str) -> Result<Value, Value> {
        let mut err: *mut c_char = std::ptr::null_mut();
        let ptr = unsafe {
            codetopo_query(handle, c(op).as_ptr(), c(args).as_ptr(), &mut err)
        };
        if ptr.is_null() {
            Err(take_err(err))
        } else {
            assert!(err.is_null(), "err_out set on success");
            Ok(take_json(ptr))
        }
    }

    fn query(handle: *const codetopo_graph, op: &str, args: &str) -> Value {
        call_query(handle, op, args).unwrap_or_else(|e| panic!("{op} failed: {e}"))
    }

    fn query_err(handle: *const codetopo_graph, op: &str, args: &str) -> Value {
        call_query(handle, op, args).expect_err("query should fail")
    }

    fn free_graph(handle: *mut codetopo_graph) {
        unsafe { codetopo_graph_free(handle) }
    }

    /// Copy the quarry fixture's three `.py` modules into a clean
    /// repo dir — the known-answer package scope (see quarry_handle).
    fn copy_quarry_package(dir: &Path) -> PathBuf {
        let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../codetopo-extract/tests/fixture-py");
        let repo = dir.join("quarry-pkg");
        std::fs::create_dir_all(&repo).expect("create fixture copy dir");
        for name in ["__init__.py", "core.py", "shapes.py"] {
            std::fs::copy(fixture.join(name), repo.join(name)).expect("copy fixture module");
        }
        repo
    }

    /// Index the quarry fixture (F2) into a temp db through the C
    /// ABI and open the handle.
    ///
    /// The known-answer package is exactly the fixture's three `.py`
    /// modules — that is what `known_answer_py.rs` feeds the builder
    /// (26 nodes / 33 edges / 2 diagnostics). Indexing the fixture
    /// *directory* wholesale also indexes its README.md (Markdown is
    /// a supported language), yielding 29/36/2 — correct behaviour at
    /// a different scope, pinned separately in
    /// [`directory_scope_counts_the_readme_too`].
    fn quarry_handle() -> (tempfile::TempDir, PathBuf, *mut codetopo_graph) {
        let dir = tempfile::tempdir().expect("temp dir");
        let repo = copy_quarry_package(dir.path());
        let db = dir.path().join("quarry.db");
        let report = call_index(&repo, &db, Some("quarry")).expect("index succeeds");
        assert_eq!(report["nodes"], 26, "quarry report: {report}");
        assert_eq!(report["edges"], 33, "quarry report: {report}");
        assert_eq!(report["diagnostics"], 2, "quarry report: {report}");
        let handle = call_open_db(&db).expect("open succeeds");
        (dir, db, handle)
    }

    #[test]
    fn version_and_schema_version_come_from_the_core() {
        let version = unsafe { CStr::from_ptr(codetopo_version()) }
            .to_str()
            .expect("version is UTF-8");
        // The core manifest's version at this base (0.9.x line) — the
        // build script reads it from codetopo-core's Cargo.toml, so
        // this asserts the plumbing, not a duplicated literal's luck.
        assert_eq!(version, env!("CODETOPO_CORE_VERSION"));
        assert!(version.starts_with("0.9"), "core version: {version}");
        let schema = unsafe { CStr::from_ptr(codetopo_schema_version()) }
            .to_str()
            .expect("schema version is UTF-8");
        assert_eq!(schema, codetopo_core::snapshot::SCHEMA_VERSION);
        assert_eq!(schema, "2.0");
    }

    #[test]
    fn lifecycle_index_open_query_free() {
        let (_dir, _db, handle) = quarry_handle();
        let stats = query(handle, "stats", "{}");
        assert_eq!(stats["nodes"]["total"], 26, "stats: {stats}");
        assert_eq!(stats["edges"]["total"], 33, "stats: {stats}");

        let diags = query(handle, "diagnostics", "{}");
        assert_eq!(diags["total"], 2, "diagnostics: {diags}");

        let verify = query(handle, "verify", "{}");
        assert_eq!(verify["chain_valid"], true, "verify: {verify}");

        let snap1 = query(handle, "snapshot", "{}");
        assert_eq!(snap1["schema_version"], "2.0");
        assert_eq!(snap1["nodes"].as_array().expect("nodes").len(), 26);

        let search = query(handle, "search_symbols", r#"{"query": "Engine"}"#);
        assert!(search["total"].as_u64().expect("total") >= 1, "search: {search}");

        let schema = query(handle, "describe_schema", "{}");
        assert_eq!(schema["schema_version"], "2.0");

        // F5: repeated queries are byte-identical.
        let mut err: *mut c_char = std::ptr::null_mut();
        let a = unsafe {
            codetopo_query(handle, c("stats").as_ptr(), c("{}").as_ptr(), &mut err)
        };
        let mut err2: *mut c_char = std::ptr::null_mut();
        let b = unsafe {
            codetopo_query(handle, c("stats").as_ptr(), c("{}").as_ptr(), &mut err2)
        };
        let sa = unsafe { CString::from_raw(a) };
        let sb = unsafe { CString::from_raw(b) };
        assert_eq!(sa, sb, "repeated stats queries must be byte-identical");

        free_graph(handle);
        free_graph(std::ptr::null_mut()); // no-op, must not crash
        unsafe { codetopo_free_string(std::ptr::null_mut()) }; // no-op, must not crash
    }

    #[test]
    fn directory_scope_counts_the_readme_too() {
        // The documented second F2 scoping (coordinator ruling,
        // 2026-11-09): indexing the fixture DIRECTORY also indexes
        // README.md as Markdown, so the same battery at directory
        // scope reads 29 nodes / 36 edges / 2 diagnostics. Both
        // numbers are correct behaviour of the core; the F2 headline
        // (26/33/2) is the package scope above.
        let dir = tempfile::tempdir().expect("temp dir");
        let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../codetopo-extract/tests/fixture-py");
        let db = dir.path().join("whole-dir.db");
        let report = call_index(&fixture, &db, Some("quarry")).expect("index succeeds");
        assert_eq!(report["files_indexed"], 4, "report: {report}");
        assert_eq!(report["nodes"], 29, "report: {report}");
        assert_eq!(report["edges"], 36, "report: {report}");
        assert_eq!(report["diagnostics"], 2, "report: {report}");
        let handle = call_open_db(&db).expect("open succeeds");
        let stats = query(handle, "stats", "{}");
        assert_eq!(stats["nodes"]["total"], 29, "stats: {stats}");
        assert_eq!(stats["edges"]["total"], 36, "stats: {stats}");
        free_graph(handle);
    }

    #[test]
    fn snapshot_handle_round_trip_and_verify() {
        let (_dir, _db, handle) = quarry_handle();
        let snap = query(handle, "snapshot", "{}");
        free_graph(handle);

        let snap_text = serde_json::to_string(&snap).expect("snapshot serializes");
        let loaded = call_load_snapshot(&snap_text).expect("load succeeds");

        let stats = query(loaded, "stats", "{}");
        assert_eq!(stats["nodes"]["total"], 26);
        assert_eq!(stats["edges"]["total"], 33);

        // The verify op checks the chain the snapshot carries (F4).
        let verify = query(loaded, "verify", "{}");
        assert_eq!(verify["chain_valid"], true, "verify: {verify}");

        // Snapshot of the loaded snapshot carries the same graph
        // content (nodes/edges), whatever the audit timestamps say.
        let snap2 = query(loaded, "snapshot", "{}");
        assert_eq!(snap["nodes"], snap2["nodes"]);
        assert_eq!(snap["edges"], snap2["edges"]);

        // diff of a graph against its own snapshot is empty.
        let diff_args = json!({"other_snapshot_json": snap_text}).to_string();
        let diff = query(loaded, "diff", &diff_args);
        assert_eq!(diff["added_nodes"].as_array().map(Vec::len), Some(0), "diff: {diff}");
        assert_eq!(diff["removed_nodes"].as_array().map(Vec::len), Some(0), "diff: {diff}");

        free_graph(loaded);
    }

    #[test]
    fn error_paths_are_json_and_never_panic() {
        let (_dir, _db, handle) = quarry_handle();

        // Unknown op.
        let e = query_err(handle, "nope_op", "{}");
        assert_eq!(e["code"], "invalid_param", "err: {e}");
        assert!(e["message"].as_str().expect("message").contains("nope_op"));

        // Bad args JSON.
        let e = query_err(handle, "stats", "{not json");
        assert_eq!(e["code"], "invalid_param", "err: {e}");

        // Non-object args.
        let e = query_err(handle, "stats", "[1,2]");
        assert_eq!(e["code"], "invalid_param", "err: {e}");

        // Missing required arg names the parameter.
        let e = query_err(handle, "descendants", "{}");
        assert_eq!(e["code"], "invalid_param", "err: {e}");
        assert!(e["message"].as_str().expect("message").contains("node"));

        // Unknown symbol.
        let e = query_err(handle, "get_symbol", r#"{"node": "quarry::nope"}"#);
        assert_eq!(e["code"], "unknown_symbol", "err: {e}");

        // rank: a non-`by` metric key errors and names the key.
        let e = query_err(handle, "rank", r#"{"metric": "pagerank"}"#);
        assert_eq!(e["code"], "invalid_param", "err: {e}");
        assert!(e["message"].as_str().expect("message").contains("metric"), "err: {e}");
        assert!(e["message"].as_str().expect("message").contains("'by'"), "err: {e}");

        // rank: an unknown metric value errors naming the value.
        let e = query_err(handle, "rank", r#"{"by": "vibes"}"#);
        assert_eq!(e["code"], "invalid_param", "err: {e}");
        assert!(e["message"].as_str().expect("message").contains("vibes"));

        // rank with `by` works.
        let rank = query(handle, "rank", r#"{"by": "pagerank", "limit": 5}"#);
        assert_eq!(rank["metric"], "pagerank", "rank: {rank}");

        free_graph(handle);

        // Missing database.
        let e = call_open_db(Path::new("/nonexistent/definitely-not-here.db"))
            .expect_err("missing db fails");
        assert_eq!(e["code"], "unknown_index", "err: {e}");

        // Bad snapshot JSON / unsupported schema version.
        let e = call_load_snapshot("{nope").expect_err("bad snapshot fails");
        assert_eq!(e["code"], "invalid_param", "err: {e}");

        let e = call_load_snapshot(r#"{"schema_version": "9.0", "nodes": [], "edges": []}"#)
            .expect_err("future schema fails");
        assert_eq!(e["code"], "unsupported_schema_version", "err: {e}");

        // NULL handle is an error, not a crash.
        let e = call_query(std::ptr::null(), "stats", "{}").expect_err("NULL handle fails");
        assert_eq!(e["code"], "invalid_param", "err: {e}");
    }

    #[test]
    fn header_declares_exactly_the_exported_functions() {
        // The pin: every function declared in include/codetopo.h is
        // in the checked export list, and vice versa. The C
        // conformance runner additionally links against the built
        // cdylib, so a declared-but-missing symbol fails there too.
        const EXPECTED: [&str; 8] = [
            "codetopo_version",
            "codetopo_schema_version",
            "codetopo_open_db",
            "codetopo_load_snapshot",
            "codetopo_graph_free",
            "codetopo_index",
            "codetopo_query",
            "codetopo_free_string",
        ];
        let header = include_str!("../include/codetopo.h");
        // Declarations are the non-comment lines ending in `;`.
        // Scan every `codetopo_*` token on such a line and keep the
        // ones immediately followed by `(` — a token scan, not a
        // first-match scan, because two declarations return
        // `codetopo_graph *` (the type name precedes the function
        // name on the same line; the type token is followed by a
        // space, never by `(`).
        let mut declared: Vec<&str> = Vec::new();
        for line in header.lines() {
            let line = line.trim();
            if !line.ends_with(';') || line.starts_with('*') || line.starts_with("/*") {
                continue;
            }
            let mut rest = line;
            while let Some(start) = rest.find("codetopo_") {
                let tail = &rest[start..];
                let end = tail
                    .find(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '_'))
                    .unwrap_or(tail.len());
                let name = &tail[..end];
                if tail[end..].starts_with('(') {
                    declared.push(name);
                }
                rest = &tail[end..];
            }
        }
        declared.sort_unstable();
        declared.dedup();
        let mut expected: Vec<&str> = EXPECTED.to_vec();
        expected.sort_unstable();
        assert_eq!(declared, expected, "header declarations drifted from the export list");
    }
}
