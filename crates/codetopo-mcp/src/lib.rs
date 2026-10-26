// Copyright (C) 2026 Nrupal Akolkar
// SPDX-License-Identifier: AGPL-3.0-or-later
// Part of codetopo — Owned by Nrupal Akolkar (https://nrupalakolkar.com) · Built with Muse by Meta (https://muse.ai/join) — join Muse, then redeem code 101EEK in Settings within 48 hours of joining and we both earn tokens.

//! MCP stdio surface for codetopo (Phase 2 design §9).
//!
//! This crate is transport only. Every tool is a thin adapter over
//! [`codetopo_cli`]'s library entry points and [`codetopo_core`]'s queries, so
//! the CLI, HTTP, and MCP surfaces answer the same question identically by
//! construction — §11 forbids the three from carrying their own graph logic.
//!
//! The wire contract is line-delimited JSON-RPC 2.0 on stdin/stdout, as the
//! MCP stdio transport requires. [`seal_stdout`] moves the protocol to a
//! private fd before the first call, because [`codetopo_cli::index_repo`]
//! reports progress with `println!` and one stray line mid-frame would
//! desynchronize the client.

use std::collections::BTreeMap;
use std::path::PathBuf;

use codetopo_cli::{
    describe_schema, index_repo, load_graph_from_db, node_kind_name, require_node, CliError,
    ALL_EDGE_KINDS, ALL_NODE_KINDS,
};
use codetopo_core::snapshot::Snapshot;
use codetopo_store::Store;
use serde_json::{json, Value};

pub mod stdio;

/// Default number of members a closure tool returns before truncation.
const DEFAULT_LIMIT: usize = 20;
/// Protocol version advertised in the `initialize` result.
const PROTOCOL_VERSION: &str = "2024-11-05";

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// A tool call that could not be served. Serialized to the MCP result as
/// `isError: true`; the code and prose are both carried so an agent can branch
/// on the code without parsing English.
#[derive(Debug, thiserror::Error)]
pub enum ToolError {
    /// The tool name is not in the catalogue below.
    #[error("unknown tool: {0}")]
    UnknownTool(String),
    /// A required argument is absent, not a string, or blank.
    #[error("missing or invalid parameter: {0}")]
    MissingParam(String),
    /// `db_path` is not a readable index.
    #[error("unknown index: {0}")]
    UnknownIndex(String),
    /// The closure tools were asked about a node the graph does not contain.
    #[error("unknown node: {0}")]
    UnknownNode(String),
    /// `from`/`to` are both known but no typed path joins them.
    #[error("no path from {from} to {to}")]
    NoPath { from: String, to: String },
    /// The underlying CLI or store refused the work.
    #[error(transparent)]
    Cli(#[from] CliError),
}

impl ToolError {
    /// Stable machine-readable code, paralleling the HTTP surface's flat
    /// `{"error", "message"}` body.
    fn code(&self) -> &'static str {
        match self {
            Self::UnknownTool(_) => "unknown_tool",
            Self::MissingParam(_) => "missing_param",
            Self::UnknownIndex(_) => "unknown_index",
            Self::UnknownNode(_) => "unknown_node",
            Self::NoPath { .. } => "no_path",
            Self::Cli(_) => "internal",
        }
    }

    /// The MCP result body for a failed tool call.
    pub fn to_result(&self) -> Value {
        json!({
            "isError": true,
            "content": [{
                "type": "text",
                "text": serde_json::to_string(&json!({
                    "error": self.code(),
                    "message": self.to_string(),
                }))
                .unwrap_or_else(|_| self.to_string()),
            }],
        })
    }
}

// ---------------------------------------------------------------------------
// Argument helpers
// ---------------------------------------------------------------------------

/// Read a required string argument, rejecting absent, non-string, and blank.
fn require_str<'a>(args: &'a Value, key: &str) -> Result<&'a str, ToolError> {
    match args.get(key).and_then(Value::as_str) {
        Some(s) if !s.trim().is_empty() => Ok(s),
        _ => Err(ToolError::MissingParam(key.to_string())),
    }
}

/// Read an optional string argument, treating absent as unset.
fn optional_str<'a>(args: &'a Value, key: &str) -> Result<Option<&'a str>, ToolError> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) if !s.trim().is_empty() => Ok(Some(s)),
        Some(_) => Err(ToolError::MissingParam(key.to_string())),
    }
}

/// Read an optional boolean argument, rejecting a non-boolean.
fn optional_bool(args: &Value, key: &str) -> Result<bool, ToolError> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(false),
        Some(Value::Bool(b)) => Ok(*b),
        Some(_) => Err(ToolError::MissingParam(key.to_string())),
    }
}

/// Read an optional limit, defaulting to [`DEFAULT_LIMIT`] and capping the
/// damage a runaway argument can do.
fn optional_limit(args: &Value) -> Result<usize, ToolError> {
    match args.get("limit") {
        None | Some(Value::Null) => Ok(DEFAULT_LIMIT),
        Some(Value::Number(n)) => {
            let limit = n
                .as_u64()
                .ok_or_else(|| ToolError::MissingParam("limit".to_string()))?;
            Ok(usize::try_from(limit).unwrap_or(usize::MAX))
        }
        Some(_) => Err(ToolError::MissingParam("limit".to_string())),
    }
}

/// The `db_path` argument as a path, checked for existence.
///
/// `Store::open` creates parent directories and initializes the schema, so
/// opening a missing index would otherwise succeed and hand back an empty
/// graph — indistinguishable from a repository that really has no nodes.
/// Existence is therefore asserted up front, as the HTTP surface does in
/// `handlers::require_index`.
fn require_index(args: &Value) -> Result<PathBuf, ToolError> {
    let db_path = PathBuf::from(require_str(args, "db_path")?);
    if !db_path.is_file() {
        return Err(ToolError::UnknownIndex(db_path.display().to_string()));
    }
    Ok(db_path)
}

/// Load the graph at `db_path`, or report it as not an index.
fn open_graph(args: &Value) -> Result<codetopo_core::Graph, ToolError> {
    let db_path = require_index(args)?;
    load_graph_from_db(&db_path).map_err(ToolError::Cli)
}

/// `nodes` closure shape shared by descendants, ancestors, and blast radius:
/// the truncated member list, plus `total` — the true closure size, so an
/// agent can budget its next query without asking for the whole thing.
fn closure_result(members: &[String], total: usize, expand: bool, limit: usize) -> Value {
    json!({
        "total": total,
        "returned": members.len(),
        "truncated": total > members.len(),
        "limit": limit,
        "expand": expand,
        "nodes": members,
    })
}

// ---------------------------------------------------------------------------
// Tool catalogue
// ---------------------------------------------------------------------------

/// The `tools/list` catalogue: name, title, description, behaviour
/// annotations, and JSON Schema for each tool. Arguments mirror the HTTP parameters one-for-one, so an agent that
/// has learned one surface has learned both (§9).
pub fn tool_catalog() -> Vec<Value> {
    let node_arg = |description: &str| {
        json!({
            "type": "object",
            "properties": {
                "db_path": {"type": "string", "description": "Path to the index database."},
                "node": {"type": "string", "description": description},
                "expand": {"type": "boolean", "description": "Return full node objects instead of ids. Default false."},
                "limit": {"type": "integer", "description": "Maximum members to return. Default 20."},
            },
            "required": ["db_path", "node"],
        })
    };

    vec![
        json!({
            "name": "codetopo_index",
            "title": "Index a repository",
            "description": "Index a repository into a codetopo graph database.",
            "annotations": {"readOnlyHint": false, "destructiveHint": false, "openWorldHint": false},
            "inputSchema": {
                "type": "object",
                "properties": {
                    "repo_path": {"type": "string", "description": "Path to the repository to index."},
                    "db_path": {"type": "string", "description": "Path of the database to create."},
                    "package": {"type": "string", "description": "Package name. Defaults to the repository directory name."},
                },
                "required": ["repo_path", "db_path"],
            },
        }),
        json!({
            "name": "codetopo_descendants",
            "title": "Find descendants",
            "description": "Nodes transitively reachable from a node over calls and imports edges.",
            "annotations": {"readOnlyHint": true, "destructiveHint": false, "openWorldHint": false},
            "inputSchema": node_arg("Node id to walk down from."),
        }),
        json!({
            "name": "codetopo_ancestors",
            "title": "Find ancestors",
            "description": "Nodes from which a node is transitively reachable over calls and imports edges.",
            "annotations": {"readOnlyHint": true, "destructiveHint": false, "openWorldHint": false},
            "inputSchema": node_arg("Node id to walk up from."),
        }),
        json!({
            "name": "codetopo_blast_radius",
            "title": "Compute blast radius",
            "description": "Nodes that break if a node changes: dependents, overriders, and implementers.",
            "annotations": {"readOnlyHint": true, "destructiveHint": false, "openWorldHint": false},
            "inputSchema": node_arg("Node id whose blast radius to compute."),
        }),
        json!({
            "name": "codetopo_path",
            "title": "Find the path between two nodes",
            "description": "Shortest typed path between two nodes, or the reason there is none.",
            "annotations": {"readOnlyHint": true, "destructiveHint": false, "openWorldHint": false},
            "inputSchema": {
                "type": "object",
                "properties": {
                    "db_path": {"type": "string", "description": "Path to the index database."},
                    "from": {"type": "string", "description": "Node id to start from."},
                    "to": {"type": "string", "description": "Node id to reach."},
                },
                "required": ["db_path", "from", "to"],
            },
        }),
        json!({
            "name": "codetopo_stats",
            "title": "Index statistics",
            "description": "Node and edge totals for an index, with per-kind breakdowns.",
            "annotations": {"readOnlyHint": true, "destructiveHint": false, "openWorldHint": false},
            "inputSchema": {
                "type": "object",
                "properties": {
                    "db_path": {"type": "string", "description": "Path to the index database."},
                },
                "required": ["db_path"],
            },
        }),
        json!({
            "name": "codetopo_verify",
            "title": "Verify the index audit chain",
            "description": "Verify the tamper-evident audit chain of an index.",
            "annotations": {"readOnlyHint": true, "destructiveHint": false, "openWorldHint": false},
            "inputSchema": {
                "type": "object",
                "properties": {
                    "db_path": {"type": "string", "description": "Path to the index database."},
                },
                "required": ["db_path"],
            },
        }),
        json!({
            "name": "codetopo_snapshot",
            "title": "Export a graph snapshot",
            "description": "Export the whole graph as a portable snapshot, bound to the audit head.",
            "annotations": {"readOnlyHint": true, "destructiveHint": false, "openWorldHint": false},
            "inputSchema": {
                "type": "object",
                "properties": {
                    "db_path": {"type": "string", "description": "Path to the index database."},
                },
                "required": ["db_path"],
            },
        }),
        json!({
            "name": "codetopo_diagnostics",
            "title": "Read the index diagnostics",
            "description": "The diagnostics recorded when the index was built - dependency cycles, multi-definitions, unresolved calls, and the rest - as data, exactly as the CLI prints them.",
            "annotations": {"readOnlyHint": true, "destructiveHint": false, "openWorldHint": false},
            "inputSchema": {
                "type": "object",
                "properties": {
                    "db_path": {"type": "string", "description": "Path to the index database."},
                },
                "required": ["db_path"],
            },
        }),
        json!({
            "name": "codetopo_rank",
            "title": "Rank symbols by a measured count",
            "description": "The top symbols by fan-in, fan-out, blast-radius size, or cyclomatic complexity - counts over the graph, with the count's method stated in the result. A ranking of measurements, never a score of code quality.",
            "annotations": {"readOnlyHint": true, "destructiveHint": false, "openWorldHint": false},
            "inputSchema": {
                "type": "object",
                "properties": {
                    "db_path": {"type": "string", "description": "Path to the index database."},
                    "by": {"type": "string", "description": "Metric to rank by: fan_in, fan_out, blast_radius (default), or complexity."},
                    "limit": {"type": "integer", "description": "Maximum entries to return. Default 20."},
                },
                "required": ["db_path"],
            },
        }),
        json!({
            "name": "codetopo_diff",
            "title": "Diff two indexes (change impact)",
            "description": "Compare two indexes (or two snapshot files): added, removed, and changed nodes and edges, plus the blast-radius movement of every touched symbol. Give exactly one before source and one after source.",
            "annotations": {"readOnlyHint": true, "destructiveHint": false, "openWorldHint": false},
            "inputSchema": {
                "type": "object",
                "properties": {
                    "before_db_path": {"type": "string", "description": "Path to the before index database."},
                    "after_db_path": {"type": "string", "description": "Path to the after index database."},
                    "before_snapshot": {"type": "string", "description": "Path to the before snapshot file (alternative to before_db_path)."},
                    "after_snapshot": {"type": "string", "description": "Path to the after snapshot file (alternative to after_db_path)."},
                },
                "required": [],
            },
        }),
        json!({
            "name": "codetopo_metrics",
            "title": "Coupling and cohesion metrics",
            "description": "Coupling and cohesion measurements for every type and package (CODE_SCHEMA section 9): CBO, RFC, LCOM pair counts, TCC, ATFD, WMC per type; Ca, Ce, instability, and separately-named external coupling per package. Every metric's method is stated in the result. Measurements only - no thresholds, no verdicts.",
            "annotations": {"readOnlyHint": true, "destructiveHint": false, "openWorldHint": false},
            "inputSchema": {
                "type": "object",
                "properties": {
                    "db_path": {"type": "string", "description": "Path to the index database."},
                },
                "required": ["db_path"],
            },
        }),
        json!({
            "name": "codetopo_neighborhood",
            "title": "Get a node's neighborhood",
            "description": "The subgraph within a few hops of a node, in both directions (contains/defines excluded).",
            "annotations": {"readOnlyHint": true, "destructiveHint": false, "openWorldHint": false},
            "inputSchema": {
                "type": "object",
                "properties": {
                    "db_path": {"type": "string", "description": "Path to the index database."},
                    "node": {"type": "string", "description": "Node id at the center."},
                    "depth": {"type": "integer", "description": "Hops to walk (default 1)."},
                },
                "required": ["db_path", "node"],
            },
        }),
        json!({
            "name": "codetopo_get_symbol",
            "title": "Get one symbol in full",
            "description": "One node in full - kind, location, tier, defines and refs - plus its incident edges grouped by kind and direction, with counts (containment excluded). Errors when the id is unknown.",
            "annotations": {"readOnlyHint": true, "destructiveHint": false, "openWorldHint": false},
            "inputSchema": {
                "type": "object",
                "properties": {
                    "db_path": {"type": "string", "description": "Path to the index database."},
                    "node": {"type": "string", "description": "Node id to fetch."},
                },
                "required": ["db_path", "node"],
            },
        }),
        json!({
            "name": "codetopo_search_symbols",
            "title": "Search symbols by name",
            "description": "Find symbols by name (case-insensitive substring of the label) - the discovery step before the exact-id queries.",
            "annotations": {"readOnlyHint": true, "destructiveHint": false, "openWorldHint": false},
            "inputSchema": {
                "type": "object",
                "properties": {
                    "db_path": {"type": "string", "description": "Path to the index database."},
                    "query": {"type": "string", "description": "Name fragment to search for."},
                    "limit": {"type": "integer", "description": "Maximum results (default 20)."},
                },
                "required": ["db_path", "query"],
            },
        }),
        json!({
            "name": "codetopo_describe_schema",
            "title": "Describe the schema",
            "description": "The schema this server speaks: node and edge kinds, tier rules, and the query vocabulary. Needs no database.",
            "annotations": {"readOnlyHint": true, "destructiveHint": false, "openWorldHint": false},
            "inputSchema": {"type": "object", "properties": {}, "required": []},
        }),
    ]
}

// ---------------------------------------------------------------------------
// Tool dispatch
// ---------------------------------------------------------------------------

/// Invoke one tool by name and return its JSON result.
///
/// Every adapter is a translation of arguments into a `codetopo-cli` or
/// `codetopo-core` call. None of them decides what a query means.
pub fn call_tool(name: &str, args: &Value) -> Result<Value, ToolError> {
    match name {
        "codetopo_index" => tool_index(args),
        "codetopo_descendants" => tool_closure(args, Closure::Descendants),
        "codetopo_ancestors" => tool_closure(args, Closure::Ancestors),
        "codetopo_blast_radius" => tool_closure(args, Closure::BlastRadius),
        "codetopo_path" => tool_path(args),
        "codetopo_stats" => tool_stats(args),
        "codetopo_verify" => tool_verify(args),
        "codetopo_snapshot" => tool_snapshot(args),
        "codetopo_diagnostics" => tool_diagnostics(args),
        "codetopo_rank" => tool_rank(args),
        "codetopo_diff" => tool_diff(args),
        "codetopo_metrics" => tool_metrics(args),
        "codetopo_neighborhood" => tool_neighborhood(args),
        "codetopo_get_symbol" => tool_get_symbol(args),
        "codetopo_search_symbols" => tool_search_symbols(args),
        "codetopo_describe_schema" => Ok(describe_schema()),
        other => Err(ToolError::UnknownTool(other.to_string())),
    }
}

/// Which core query a closure tool wraps. The three share an argument shape
/// and a result shape; only the traversal differs.
#[derive(Debug, Clone, Copy)]
enum Closure {
    Descendants,
    Ancestors,
    BlastRadius,
}

fn tool_index(args: &Value) -> Result<Value, ToolError> {
    let repo_path = PathBuf::from(require_str(args, "repo_path")?);
    let db_path = PathBuf::from(require_str(args, "db_path")?);
    // An unnamed package is the repository itself: the directory name is the
    // only identity the caller did not have to invent.
    let package = match optional_str(args, "package")? {
        Some(pkg) => pkg.to_string(),
        None => codetopo_cli::default_package_name(&repo_path),
    };

    // Snapshots and HMAC proofs are out of scope for the *index* step on the
    // stdio surface: the
    // local MCP client is trusted with the filesystem, and §6 gives the
    // authenticated HTTP surface the tamper-evident audit chain.
    let report = index_repo(&repo_path, &db_path, &package, None, None)?;
    Ok(json!({
        "package": package,
        "files_indexed": report.files_indexed,
        "files_failed": report.files_failed,
        "nodes": report.nodes,
        "edges": report.edges,
        "diagnostics": report.diagnostics,
    }))
}

fn tool_closure(args: &Value, which: Closure) -> Result<Value, ToolError> {
    let node = require_str(args, "node")?.to_string();
    let expand = optional_bool(args, "expand")?;
    let limit = optional_limit(args)?;
    let graph = open_graph(args)?;

    // The node is checked here, not left to the walk: an empty closure for a
    // typo'd id is indistinguishable from a leaf, and §3 treats "no such node"
    // as an error precisely so that cannot happen. The CLI's `UnknownId` is
    // re-typed so the tool reports `unknown_node`, not an internal failure.
    require_node(&graph, &node).map_err(|_| ToolError::UnknownNode(node.clone()))?;

    let members: Vec<String> = match which {
        Closure::Descendants => graph.descendants(&node),
        Closure::Ancestors => graph.ancestors(&node),
        Closure::BlastRadius => graph.blast_radius(&node),
    };
    let total = members.len();
    let shown: Vec<String> = members.into_iter().take(limit).collect();

    if !expand {
        return Ok(closure_result(&shown, total, expand, limit));
    }

    // Expanded members are serialized by the core node type, so a tool
    // response and `GET /v1/graphs/{id}/snapshot` cannot spell a node two
    // ways. An id that survived `require_node` but is not in this graph is
    // impossible, hence the `filter_map`.
    let expanded: Vec<Value> = shown
        .iter()
        .filter_map(|id| {
            graph
                .get(id)
                .and_then(|node| serde_json::to_value(node).ok())
        })
        .collect();
    let mut result = closure_result(&[], total, expand, limit);
    result["returned"] = json!(expanded.len());
    result["truncated"] = json!(total > expanded.len());
    result["nodes"] = json!(expanded);
    Ok(result)
}

fn tool_path(args: &Value) -> Result<Value, ToolError> {
    let from = require_str(args, "from")?.to_string();
    let to = require_str(args, "to")?.to_string();
    let graph = open_graph(args)?;

    require_node(&graph, &from).map_err(|_| ToolError::UnknownNode(from.clone()))?;
    require_node(&graph, &to).map_err(|_| ToolError::UnknownNode(to.clone()))?;

    // A partial path is not a meaningful object, so the full typed path is
    // returned or an error is — never a truncated one (§8).
    let steps = graph.path(&from, &to).ok_or_else(|| ToolError::NoPath {
        from: from.clone(),
        to: to.clone(),
    })?;

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

    Ok(json!({
        "path": steps,
        "nodes": nodes,
        "edges": edges,
    }))
}

fn tool_stats(args: &Value) -> Result<Value, ToolError> {
    let graph = open_graph(args)?;

    // Kind names and tally order come from the CLI, so `codetopo_stats` and
    // `codetopo stats` break the same kinds down under the same names.
    let mut node_kinds: BTreeMap<&str, usize> = BTreeMap::new();
    for kind in ALL_NODE_KINDS {
        let count = graph.nodes().filter(|node| node.kind == kind).count();
        if count > 0 {
            node_kinds.insert(node_kind_name(kind), count);
        }
    }
    let mut edge_kinds: BTreeMap<&str, usize> = BTreeMap::new();
    for kind in ALL_EDGE_KINDS {
        let count = graph.edge_count_by_kind(kind);
        if count > 0 {
            edge_kinds.insert(kind.as_str(), count);
        }
    }

    Ok(json!({
        "nodes": graph.node_count(),
        "edges": graph.edge_count(),
        "node_kinds": node_kinds,
        "edge_kinds": edge_kinds,
    }))
}

fn tool_verify(args: &Value) -> Result<Value, ToolError> {
    let db_path = require_index(args)?;
    // Store errors are wrapped rather than re-typed: the file is known to
    // exist by now, so anything that goes wrong here is a real failure rather
    // than a missing index.
    let store = Store::open(&db_path).map_err(store_failure)?;
    let audit = store.load_audit().map_err(store_failure)?;
    Ok(json!({
        "entries": audit.len(),
        "chain_valid": audit.verify(),
        "head": audit.head(),
    }))
}

/// Re-type a store failure as the CLI error the `Cli` variant carries.
fn store_failure(err: codetopo_store::StoreError) -> ToolError {
    ToolError::Cli(CliError::Store(err))
}

/// `codetopo_snapshot` — portable frozen export of the whole graph.
///
/// Mirrors the HTTP `GET /v1/graphs/{id}/snapshot` body exactly: the
/// [`Snapshot`] struct itself, unwrapped, so an agent can hand the result to
/// `codetopo restore` without a translation step. The `audit_head` is read from
/// the same audit log `codetopo_verify` reports on, so the two agree.
fn tool_snapshot(args: &Value) -> Result<Value, ToolError> {
    let db_path = require_index(args)?;
    let store = Store::open(&db_path).map_err(store_failure)?;
    let graph = store.load_graph().map_err(store_failure)?;
    let audit = store.load_audit().map_err(store_failure)?;
    let snapshot = Snapshot::from_graph(&graph, &audit.snapshot_entries());
    serde_json::to_value(snapshot).map_err(|err| ToolError::MissingParam(err.to_string()))
}

// ---------------------------------------------------------------------------
// JSON-RPC envelope
// ---------------------------------------------------------------------------

/// A JSON-RPC response to write back, if the request warrants one.
pub fn handle_request(request: &Value) -> Option<Value> {
    // A batch array is not part of the stdio contract and is refused rather
    // than partially served.
    if request.is_array() {
        return Some(error_response(
            Value::Null,
            -32600,
            "batch requests are not supported",
        ));
    }
    if !request.is_object() {
        return Some(error_response(Value::Null, -32700, "invalid request"));
    }

    // A missing `jsonrpc` is tolerated: every client in the wild sends
    // `"2.0"`, and refusing a well-formed method call over a missing
    // boilerplate field helps nobody.
    let id = request.get("id").cloned().unwrap_or(Value::Null);
    let method = request.get("method").and_then(Value::as_str);
    let Some(method) = method else {
        return Some(error_response(
            id,
            -32600,
            "invalid request: missing method",
        ));
    };

    // Notifications carry no `id` and are answered with silence.
    let _ = request.get("id")?;

    let params = request.get("params").cloned().unwrap_or_else(|| json!({}));
    match method {
        "initialize" => Some(json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {
                "protocolVersion": PROTOCOL_VERSION,
                "capabilities": {"tools": {}},
                "serverInfo": {"name": "codetopo", "version": env!("CARGO_PKG_VERSION")},
            },
        })),
        "tools/list" => Some(json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {"tools": tool_catalog()},
        })),
        "tools/call" => Some(handle_tool_call(&id, &params)),
        "ping" => Some(json!({"jsonrpc": "2.0", "id": id, "result": {}})),
        other => Some(error_response(
            id,
            -32601,
            &format!("method not found: {other}"),
        )),
    }
}

fn tool_neighborhood(args: &Value) -> Result<Value, ToolError> {
    let graph = open_graph(args)?;
    let node = require_str(args, "node")?;
    require_node(&graph, node).map_err(|_| ToolError::UnknownNode(node.to_string()))?;
    let depth = args.get("depth").and_then(Value::as_u64).unwrap_or(1) as u32;
    let sub = graph.neighborhood(node, depth);
    Ok(json!({
        "node": node,
        "depth": depth,
        "nodes": sub.nodes,
        "edges": sub.edges,
    }))
}

fn tool_get_symbol(args: &Value) -> Result<Value, ToolError> {
    let graph = open_graph(args)?;
    let node_id = require_str(args, "node")?;
    // The DESIGN §3 dossier, serialized by the core type itself — the same
    // object the CLI prints and the HTTP symbol route returns.
    let dossier = graph
        .dossier(node_id)
        .ok_or_else(|| ToolError::UnknownNode(node_id.to_string()))?;
    serde_json::to_value(dossier).map_err(|e| ToolError::Cli(CliError::Json(e)))
}

/// `codetopo_metrics` — coupling and cohesion measurements for every
/// type and package, in the shared shape the CLI's `metrics --json`
/// and the HTTP metrics route also return.
fn tool_metrics(args: &Value) -> Result<Value, ToolError> {
    let graph = open_graph(args)?;
    Ok(codetopo_cli::metrics_value(&graph))
}

/// `codetopo_diagnostics` — the diagnostics recorded when the index was
/// built, in the shared `{total, diagnostics}` shape the CLI's
/// `diagnostics --json` and the HTTP diagnostics route also return.
fn tool_diagnostics(args: &Value) -> Result<Value, ToolError> {
    let db_path = require_index(args)?;
    codetopo_cli::diagnostics_value(&db_path).map_err(ToolError::Cli)
}

/// `codetopo_diff` — the change-impact diff of two indexes (or two
/// snapshot files), in the shared shape the CLI's `diff --json` and
/// the HTTP diff route also return. Exactly one before source and one
/// after source, the CLI command's rule.
fn tool_diff(args: &Value) -> Result<Value, ToolError> {
    let load = |db_key: &str, snap_key: &str| -> Result<codetopo_core::Graph, ToolError> {
        match (optional_str(args, db_key)?, optional_str(args, snap_key)?) {
            (Some(db), None) => {
                let path = PathBuf::from(db);
                if !path.is_file() {
                    return Err(ToolError::UnknownIndex(db.to_string()));
                }
                load_graph_from_db(&path).map_err(ToolError::Cli)
            }
            (None, Some(snap)) => {
                codetopo_cli::read_snapshot_graph(&PathBuf::from(snap)).map_err(ToolError::Cli)
            }
            _ => Err(ToolError::MissingParam(format!("{db_key} / {snap_key}"))),
        }
    };
    let before = load("before_db_path", "before_snapshot")?;
    let after = load("after_db_path", "after_snapshot")?;
    Ok(codetopo_cli::diff_value(&before, &after))
}

/// `codetopo_rank` — the top symbols by a measured count, in the shared
/// `{metric, method, total, results}` shape the CLI's `rank --json` and
/// the HTTP rank route also return.
fn tool_rank(args: &Value) -> Result<Value, ToolError> {
    let graph = open_graph(args)?;
    let by = optional_str(args, "by")?.unwrap_or("blast_radius");
    let metric = codetopo_core::RankMetric::parse(by)
        .ok_or_else(|| ToolError::MissingParam("by".to_string()))?;
    let limit = optional_limit(args)?;
    Ok(codetopo_cli::rank_value(&graph, metric, limit))
}

fn tool_search_symbols(args: &Value) -> Result<Value, ToolError> {
    let graph = open_graph(args)?;
    let query = require_str(args, "query")?;
    let limit = args.get("limit").and_then(Value::as_u64).unwrap_or(20) as usize;
    let hits = graph.search_symbols(query, limit);
    Ok(json!({"query": query, "total": hits.len(), "results": hits}))
}

/// `tools/call`, including the parse-error case where there is no usable id.
fn handle_tool_call(id: &Value, params: &Value) -> Value {
    let name = match params.get("name").and_then(Value::as_str) {
        Some(name) => name,
        None => return error_response(id.clone(), -32602, "missing tool name"),
    };
    let args = params
        .get("arguments")
        .cloned()
        .unwrap_or_else(|| json!({}));

    match call_tool(name, &args) {
        // A successful CallToolResult carries `content` (what strict
        // MCP clients parse — the SDK result schemas require it) plus
        // `structuredContent` with the same object for clients that
        // consume it directly. Returning the bare object, as this
        // server once did, parses as NO content in the official
        // SDKs — in-house tests were green while real clients
        // failed, which is exactly what the stdio conformance test
        // (tests/stdio_strict.rs) now pins down.
        Ok(result) => {
            let text = serde_json::to_string(&result).unwrap_or_else(|_| "{}".to_string());
            json!({"jsonrpc": "2.0", "id": id, "result": {
                "content": [{"type": "text", "text": text}],
                "structuredContent": result,
                "isError": false,
            }})
        }
        // A tool that cannot answer is a *result* with `isError`, per MCP:
        // the model should see the prose and retry, not the transport.
        Err(err) => json!({"jsonrpc": "2.0", "id": id, "result": err.to_result()}),
    }
}

/// A JSON-RPC error object.
fn error_response(id: Value, code: i64, message: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
}

/// One line of input to one line of output: `None` when the request deserves
/// no response (a notification), `Some` for everything else including a
/// malformed line, which is answered with a null-id `-32700` so the client
/// learns its frame was wrong instead of waiting for a reply.
pub fn handle_line(line: &str) -> Option<Value> {
    match serde_json::from_str::<Value>(line) {
        Ok(request) => handle_request(&request),
        Err(err) => Some(error_response(
            Value::Null,
            -32700,
            &format!("parse error: {err}"),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};

    /// The fixture repository: a library crate whose one chain runs
    /// `handle -> query -> connect -> send`, so a path between the first and
    /// last function has exactly one answer.
    fn fixture_repo() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixture-repo")
    }

    /// A real index of the fixture repository, in a temp dir that outlives it.
    fn fixture_index() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().expect("temp dir");
        let db_path = dir.path().join("fixture.db");
        let report = index_repo(&fixture_repo(), &db_path, "fixture", None, None)
            .expect("fixture repo indexes");
        assert!(report.files_indexed > 0, "fixture repo indexed no files");
        (dir, db_path)
    }

    /// Any function node id in the fixture graph.
    fn a_function_id(db_path: &Path) -> String {
        let graph = load_graph_from_db(db_path).expect("graph loads");
        let id = graph
            .nodes()
            .find(|node| node.kind == codetopo_core::NodeKind::Function)
            .map(|node| node.id.clone())
            .expect("fixture has a function node");
        id
    }

    fn args(db_path: &Path, extra: Value) -> Value {
        let mut args = json!({"db_path": db_path.display().to_string()});
        for (key, value) in extra.as_object().expect("object") {
            args[key] = value.clone();
        }
        args
    }

    fn result_of(name: &str, args: &Value) -> Value {
        match call_tool(name, args) {
            Ok(value) => value,
            Err(err) => panic!("{name} failed: {err}"),
        }
    }

    // -- catalogue ---------------------------------------------------------

    #[test]
    fn catalogue_lists_sixteen_tools() {
        let catalog = tool_catalog();
        assert_eq!(catalog.len(), 16, "expected 16 tools: {catalog:?}");

        let names: Vec<&str> = catalog
            .iter()
            .map(|tool| tool["name"].as_str().expect("tool name"))
            .collect();
        assert_eq!(
            names,
            vec![
                "codetopo_index",
                "codetopo_descendants",
                "codetopo_ancestors",
                "codetopo_blast_radius",
                "codetopo_path",
                "codetopo_stats",
                "codetopo_verify",
                "codetopo_snapshot",
                "codetopo_diagnostics",
                "codetopo_rank",
                "codetopo_diff",
                "codetopo_metrics",
                "codetopo_neighborhood",
                "codetopo_get_symbol",
                "codetopo_search_symbols",
                "codetopo_describe_schema",
            ]
        );
        for tool in &catalog {
            assert!(tool["description"].is_string(), "missing description");
            assert_eq!(tool["inputSchema"]["type"], "object");
            assert!(tool["inputSchema"]["required"].is_array());
        }
    }

    #[test]
    fn unknown_tool_is_refused() {
        let err = call_tool("codetopo_nope", &json!({})).expect_err("unknown tool");
        assert!(matches!(err, ToolError::UnknownTool(_)));
        assert_eq!(err.code(), "unknown_tool");
    }

    // -- dispatch ----------------------------------------------------------

    #[test]
    fn dispatches_each_tool_in_the_catalogue() {
        let (_dir, db_path) = fixture_index();
        let node = a_function_id(&db_path);
        let graph = load_graph_from_db(&db_path).expect("graph loads");
        let other = graph
            .nodes()
            .find(|n| n.id != node && n.kind == codetopo_core::NodeKind::Function)
            .map(|n| n.id.clone())
            .expect("a second function node");

        for name in [
            "codetopo_descendants",
            "codetopo_ancestors",
            "codetopo_blast_radius",
        ] {
            let value = result_of(name, &args(&db_path, json!({"node": node})));
            assert!(value["total"].is_number(), "{name}: no total");
            assert!(value["nodes"].is_array(), "{name}: no nodes");
        }
        result_of("codetopo_stats", &args(&db_path, json!({})));
        result_of("codetopo_verify", &args(&db_path, json!({})));
        let snapshot = result_of("codetopo_snapshot", &args(&db_path, json!({})));
        assert!(snapshot["nodes"].is_array(), "no nodes in snapshot");
        assert!(snapshot["audit_head"].is_string(), "no audit head");
        let path = result_of(
            "codetopo_path",
            &args(&db_path, json!({"from": node, "to": other})),
        );
        assert!(path["path"].is_array());
    }

    // -- index -------------------------------------------------------------

    #[test]
    fn index_defaults_package_to_the_repo_directory_name() {
        let dir = tempfile::tempdir().expect("temp dir");
        let db_path = dir.path().join("indexed.db");
        let value = result_of(
            "codetopo_index",
            &json!({
                "repo_path": fixture_repo().display().to_string(),
                "db_path": db_path.display().to_string(),
            }),
        );
        assert_eq!(value["package"], "fixture-repo", "package not defaulted");
        assert!(value["nodes"].as_u64().expect("nodes") > 0);
        assert!(value["edges"].as_u64().expect("edges") > 0);
    }

    #[test]
    fn index_requires_its_paths() {
        let err = call_tool("codetopo_index", &json!({})).expect_err("no args");
        assert!(matches!(err, ToolError::MissingParam(_)));
        assert_eq!(err.code(), "missing_param");
    }

    // -- closures ----------------------------------------------------------

    #[test]
    fn closure_defaults_to_twenty_ids() {
        let (_dir, db_path) = fixture_index();
        let node = a_function_id(&db_path);
        let value = result_of(
            "codetopo_descendants",
            &args(&db_path, json!({"node": node})),
        );

        assert_eq!(value["limit"], DEFAULT_LIMIT);
        assert_eq!(value["expand"], false);
        assert!(value["nodes"].is_array());
        for id in value["nodes"].as_array().expect("nodes") {
            assert!(id.is_string(), "unexpanded members must be ids: {id}");
        }
    }

    #[test]
    fn closure_expands_to_node_objects() {
        let (_dir, db_path) = fixture_index();
        let node = a_function_id(&db_path);
        let value = result_of(
            "codetopo_descendants",
            &args(&db_path, json!({"node": node, "expand": true})),
        );
        assert_eq!(value["expand"], true);
        for member in value["nodes"].as_array().expect("nodes") {
            assert!(member["id"].is_string(), "expanded member has no id");
            assert!(member["kind"].is_string(), "expanded member has no kind");
        }
    }

    #[test]
    fn closure_truncates_at_the_limit_and_reports_the_true_total() {
        let (_dir, db_path) = fixture_index();
        let node = a_function_id(&db_path);
        let full = result_of(
            "codetopo_descendants",
            &args(&db_path, json!({"node": node})),
        );
        let total = full["total"].as_u64().expect("total");

        let capped = result_of(
            "codetopo_descendants",
            &args(&db_path, json!({"node": node, "limit": 1})),
        );
        assert_eq!(
            capped["total"], total,
            "total must be the true closure size"
        );
        assert_eq!(capped["returned"], 1);
        assert_eq!(capped["truncated"], total > 1);
        assert_eq!(
            capped["nodes"].as_array().expect("nodes").len(),
            1,
            "limit must cap the member list"
        );
    }

    #[test]
    fn closure_refuses_an_unknown_node() {
        let (_dir, db_path) = fixture_index();
        let err = call_tool(
            "codetopo_descendants",
            &args(&db_path, json!({"node": "does::not::exist"})),
        )
        .expect_err("unknown node");
        assert!(matches!(err, ToolError::UnknownNode(_)));
        assert!(err.to_string().contains("unknown node"), "{err}");
    }

    #[test]
    fn closure_requires_a_node() {
        let (_dir, db_path) = fixture_index();
        let err = call_tool("codetopo_ancestors", &args(&db_path, json!({}))).expect_err("no node");
        assert!(matches!(err, ToolError::MissingParam(_)));
    }

    // -- path --------------------------------------------------------------

    #[test]
    fn path_returns_the_typed_chain() {
        let (_dir, db_path) = fixture_index();
        let graph = load_graph_from_db(&db_path).expect("graph loads");
        // Walk a real calls edge in its true direction to get a pair
        // that is joined. (An earlier version of this test walked the
        // edge backwards and expected a chain — that expectation
        // enshrined the reversed-path fabrication fixed in core.)
        let edge = graph
            .edges()
            .find(|e| e.kind == codetopo_core::EdgeKind::Calls)
            .expect("fixture has a calls edge");
        let value = result_of(
            "codetopo_path",
            &args(&db_path, json!({"from": edge.from, "to": edge.to})),
        );
        let steps = value["path"].as_array().expect("path");
        assert!(!steps.is_empty());
        assert!(steps[0]["kind"].is_string(), "steps must be typed");
        assert_eq!(steps[0]["from"], edge.from);
        assert_eq!(steps[steps.len() - 1]["to"], edge.to);
        // The backwards query has no directed path and must say so.
        let err = call_tool(
            "codetopo_path",
            &args(&db_path, json!({"from": edge.to, "to": edge.from})),
        )
        .expect_err("no directed path backwards");
        assert!(matches!(err, ToolError::NoPath { .. }), "unexpected: {err}");
    }

    #[test]
    fn path_reports_when_there_is_none() {
        let (_dir, db_path) = fixture_index();
        let graph = load_graph_from_db(&db_path).expect("graph loads");
        let mut ids = graph.nodes().map(|n| n.id.clone());
        let from = ids.next().expect("a node");
        let to = ids.find(|id| *id != from).expect("another node");
        let err = call_tool(
            "codetopo_path",
            &args(&db_path, json!({"from": from, "to": to})),
        )
        .expect_err("unrelated nodes");
        assert!(
            matches!(err, ToolError::NoPath { .. } | ToolError::UnknownNode(_)),
            "unexpected error: {err}"
        );
    }

    // -- stats and verify --------------------------------------------------

    #[test]
    fn stats_totals_and_non_zero_kinds_only() {
        let (_dir, db_path) = fixture_index();
        let graph = load_graph_from_db(&db_path).expect("graph loads");
        let value = result_of("codetopo_stats", &args(&db_path, json!({})));

        assert_eq!(value["nodes"], graph.node_count());
        assert_eq!(value["edges"], graph.edge_count());

        let node_sum: u64 = value["node_kinds"]
            .as_object()
            .expect("node_kinds")
            .values()
            .map(|n| n.as_u64().expect("count"))
            .sum();
        let edge_sum: u64 = value["edge_kinds"]
            .as_object()
            .expect("edge_kinds")
            .values()
            .map(|n| n.as_u64().expect("count"))
            .sum();
        assert_eq!(
            node_sum,
            graph.node_count() as u64,
            "kinds must sum to total"
        );
        assert_eq!(
            edge_sum,
            graph.edge_count() as u64,
            "kinds must sum to total"
        );

        for count in value["node_kinds"].as_object().expect("kinds").values() {
            assert!(
                count.as_u64().expect("count") > 0,
                "zero kinds must be omitted"
            );
        }
        for count in value["edge_kinds"].as_object().expect("kinds").values() {
            assert!(
                count.as_u64().expect("count") > 0,
                "zero kinds must be omitted"
            );
        }
        assert!(
            value["edge_kinds"].get("contains").is_some(),
            "kind names must be the CLI's wire names"
        );
    }

    #[test]
    fn verify_reports_a_valid_chain() {
        let (_dir, db_path) = fixture_index();
        let value = result_of("codetopo_verify", &args(&db_path, json!({})));
        assert_eq!(value["chain_valid"], true);
        assert!(value["entries"].as_u64().expect("entries") >= 1);
        assert!(!value["head"].as_str().expect("head").is_empty());
    }

    #[test]
    fn snapshot_carries_the_whole_graph_and_the_audit_head() {
        let (_dir, db_path) = fixture_index();
        let graph = load_graph_from_db(&db_path).expect("graph loads");
        let value = result_of("codetopo_snapshot", &args(&db_path, json!({})));

        assert_eq!(
            value["schema_version"].as_str().expect("schema"),
            codetopo_core::snapshot::SCHEMA_VERSION
        );
        assert_eq!(
            value["nodes"].as_array().expect("nodes").len(),
            graph.node_count()
        );
        assert_eq!(
            value["edges"].as_array().expect("edges").len(),
            graph.edge_count()
        );

        // The head must be the one `codetopo_verify` reports: a snapshot that
        // claimed a different chain would be a different artifact.
        let verified = result_of("codetopo_verify", &args(&db_path, json!({})));
        assert_eq!(value["audit_head"], verified["head"]);

        // Edges are sorted by (from, to, kind) for a byte-stable export.
        let edges = value["edges"].as_array().expect("edges");
        let keys: Vec<(String, String, String)> = edges
            .iter()
            .map(|e| {
                (
                    e["from"].as_str().expect("from").to_string(),
                    e["to"].as_str().expect("to").to_string(),
                    e["kind"].as_str().expect("kind").to_string(),
                )
            })
            .collect();
        let mut sorted = keys.clone();
        sorted.sort();
        assert_eq!(keys, sorted, "snapshot edges must be sorted");
    }

    // -- error surface -----------------------------------------------------

    #[test]
    fn unknown_index_is_not_a_generic_failure() {
        let dir = tempfile::tempdir().expect("temp dir");
        let missing = dir.path().join("nope.db");
        let err =
            call_tool("codetopo_stats", &args(&missing, json!({}))).expect_err("missing index");
        assert!(
            matches!(err, ToolError::UnknownIndex(_) | ToolError::Cli(_)),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn a_failed_call_is_a_result_with_is_error() {
        let body = ToolError::UnknownNode("x".into()).to_result();
        assert_eq!(body["isError"], true);
        let text = body["content"][0]["text"].as_str().expect("text");
        let parsed: Value = serde_json::from_str(text).expect("code and prose");
        assert_eq!(parsed["error"], "unknown_node");
        assert!(parsed["message"]
            .as_str()
            .expect("message")
            .contains("unknown node"));
    }

    // -- JSON-RPC envelope -------------------------------------------------

    #[test]
    fn initialize_advertises_tools() {
        let response =
            handle_line(r#"{"jsonrpc":"2.0","id":1,"method":"initialize"}"#).expect("a response");
        assert_eq!(response["id"], 1);
        assert_eq!(response["result"]["protocolVersion"], PROTOCOL_VERSION);
        assert!(response["result"]["capabilities"]["tools"].is_object());
    }

    #[test]
    fn tools_list_returns_the_catalogue() {
        let response =
            handle_line(r#"{"jsonrpc":"2.0","id":7,"method":"tools/list"}"#).expect("a response");
        assert_eq!(response["id"], 7);
        assert_eq!(
            response["result"]["tools"].as_array().expect("tools").len(),
            16
        );
    }

    #[test]
    fn notifications_get_no_reply() {
        assert!(handle_line(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#).is_none());
    }

    #[test]
    fn malformed_json_is_a_parse_error_with_a_null_id() {
        let response = handle_line("{not json").expect("a response");
        assert!(response["id"].is_null());
        assert_eq!(response["error"]["code"], -32700);
    }

    #[test]
    fn unknown_method_is_method_not_found() {
        let response = handle_line(r#"{"jsonrpc":"2.0","id":2,"method":"resources/list"}"#)
            .expect("a response");
        assert_eq!(response["id"], 2);
        assert_eq!(response["error"]["code"], -32601);
    }

    #[test]
    fn a_batch_is_refused_rather_than_partly_served() {
        let response =
            handle_line(r#"[{"jsonrpc":"2.0","id":1,"method":"ping"}]"#).expect("a response");
        assert_eq!(response["error"]["code"], -32600);
    }

    #[test]
    fn tool_call_reports_unknown_nodes_to_the_model() {
        let response = handle_line(
            r#"{"jsonrpc":"2.0","id":3,"method":"tools/call",
                "params":{"name":"codetopo_descendants",
                          "arguments":{"db_path":"/nonexistent/x.db","node":"a::b"}}}"#,
        )
        .expect("a response");
        assert_eq!(response["id"], 3);
        assert_eq!(response["result"]["isError"], true);
        let text = response["result"]["content"][0]["text"]
            .as_str()
            .expect("text");
        assert!(
            text.contains("unknown index") || text.contains("unknown node"),
            "unexpected message: {text}"
        );
    }

    #[test]
    fn a_missing_jsonrpc_field_does_not_break_a_good_call() {
        let response = handle_line(r#"{"id":9,"method":"tools/list"}"#).expect("a response");
        assert_eq!(response["id"], 9);
    }

    #[test]
    fn discovery_tools_search_fetch_and_describe() {
        let (_dir, db_path) = fixture_index();
        let node = a_function_id(&db_path);
        let graph = load_graph_from_db(&db_path).expect("graph loads");
        let label = graph.get(&node).expect("node").label.clone();

        // search finds the function by its label fragment
        let found = result_of(
            "codetopo_search_symbols",
            &args(&db_path, json!({"query": label})),
        );
        assert!(found["results"]
            .as_array()
            .expect("results")
            .iter()
            .any(|n| n["id"] == node));

        // get_symbol returns the dossier (the node under `node`, plus its
        // incident edges and counts); unknown ids are unknown_node
        let fetched = result_of("codetopo_get_symbol", &args(&db_path, json!({"node": node})));
        assert_eq!(fetched["node"]["id"], node);
        let err = call_tool(
            "codetopo_get_symbol",
            &args(&db_path, json!({"node": "no::such::id"})),
        )
        .expect_err("unknown symbol must fail");
        assert_eq!(err.code(), "unknown_node");

        // neighborhood contains its center
        let hood = result_of(
            "codetopo_neighborhood",
            &args(&db_path, json!({"node": node, "depth": 1})),
        );
        assert!(hood["nodes"]
            .as_array()
            .expect("nodes")
            .iter()
            .any(|n| n["id"] == node));

        // describe_schema needs no database at all
        let schema = result_of("codetopo_describe_schema", &json!({}));
        assert_eq!(schema["schema_version"], "1.1");
        assert!(schema["node_kinds"]
            .as_array()
            .expect("kinds")
            .iter()
            .any(|k| k == "function"));
    }

    // -- analytics depth: diagnostics read + symbol dossier ----------------

    /// A planted repository — a mutual call (a cycle), a duplicated
    /// definition, and an unresolved call — indexed through the tool
    /// itself. Returns the temp dir (kept alive by the caller) and the
    /// database path.
    fn planted_index() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().expect("temp dir");
        let repo = dir.path().join("planted");
        std::fs::create_dir_all(&repo).expect("repo dir");
        std::fs::write(
            repo.join("cyc.rs"),
            "pub fn a() {\n    b();\n    missing_fn();\n}\n\npub fn b() {\n    a();\n}\n",
        )
        .expect("write cyc.rs");
        std::fs::write(repo.join("dup.rs"), "pub fn d() {}\n\npub fn d() {}\n")
            .expect("write dup.rs");
        let db_path = dir.path().join("planted.db");
        let report = result_of(
            "codetopo_index",
            &json!({
                "repo_path": repo.display().to_string(),
                "db_path": db_path.display().to_string(),
                "package": "planted",
            }),
        );
        assert_eq!(report["diagnostics"], 3, "planted index report: {report}");
        (dir, db_path)
    }

    #[test]
    fn diagnostics_tool_returns_exactly_the_cli_list() {
        let (_dir, db_path) = planted_index();
        let via_tool = result_of("codetopo_diagnostics", &args(&db_path, json!({})));
        let via_cli = codetopo_cli::diagnostics_value(&db_path).expect("cli diagnostics value");
        assert_eq!(via_tool, via_cli, "MCP diagnostics == CLI diagnostics");
        assert_eq!(via_tool["total"], 3);
        let kinds: Vec<&str> = via_tool["diagnostics"]
            .as_array()
            .expect("diagnostics array")
            .iter()
            .map(|d| {
                d.as_object()
                    .and_then(|o| o.keys().next())
                    .map(|k| k.as_str())
                    .expect("variant tag")
            })
            .collect();
        assert!(kinds.contains(&"Cycle"), "planted cycle reported: {kinds:?}");
        assert!(
            kinds.contains(&"MultiDefinition"),
            "planted duplicate reported: {kinds:?}"
        );
        assert!(
            kinds.contains(&"UnresolvedCall"),
            "planted unresolved call reported: {kinds:?}"
        );
    }

    #[test]
    fn rank_tool_matches_the_cli_value_for_every_metric() {
        let (_dir, db_path) = fixture_index();
        let graph = load_graph_from_db(&db_path).expect("graph loads");
        for (by, metric) in [
            ("fan_in", codetopo_core::RankMetric::FanIn),
            ("fan_out", codetopo_core::RankMetric::FanOut),
            ("blast_radius", codetopo_core::RankMetric::BlastRadius),
            ("complexity", codetopo_core::RankMetric::Complexity),
        ] {
            let via_tool = result_of("codetopo_rank", &args(&db_path, json!({"by": by})));
            assert_eq!(
                via_tool,
                codetopo_cli::rank_value(&graph, metric, 20),
                "MCP rank == CLI rank for {by}"
            );
        }
        // Default metric is blast_radius; an unknown metric is refused.
        let via_tool = result_of("codetopo_rank", &args(&db_path, json!({})));
        assert_eq!(via_tool["metric"], "blast_radius");
        assert!(call_tool("codetopo_rank", &args(&db_path, json!({"by": "nope"}))).is_err());
    }

    #[test]
    fn metrics_tool_matches_the_cli_value() {
        let (_dir, db_path) = fixture_index();
        let graph = load_graph_from_db(&db_path).expect("graph loads");
        let via_tool = result_of("codetopo_metrics", &args(&db_path, json!({})));
        assert_eq!(
            via_tool,
            codetopo_cli::metrics_value(&graph),
            "MCP metrics == CLI metrics for the same database"
        );
        assert!(
            via_tool["methods"]["cbo"].as_str().expect("method").contains("Chidamber"),
            "the methods travel with the numbers"
        );
    }

    #[test]
    fn diff_tool_matches_the_shared_diff_for_db_and_snapshot_pairs() {
        let dir = tempfile::tempdir().expect("temp dir");
        let write_repo = |name: &str, body: &str| {
            let repo = dir.path().join(name);
            std::fs::create_dir_all(&repo).expect("create repo dir");
            std::fs::write(repo.join("main.rs"), body).expect("write main.rs");
            repo
        };
        let repo1 = write_repo(
            "repo1",
            "pub fn a() {\n    b();\n}\n\npub fn b() {}\n\npub fn c() {}\n",
        );
        let repo2 = write_repo(
            "repo2",
            "pub fn a() {\n    b();\n}\n\npub fn b() {}\n\npub fn c() {\n    d();\n}\n\npub fn d() {}\n",
        );
        let db1 = dir.path().join("one.db");
        let db2 = dir.path().join("two.db");
        index_repo(&repo1, &db1, "pair", None, None).expect("index repo1");
        index_repo(&repo2, &db2, "pair", None, None).expect("index repo2");
        let g1 = load_graph_from_db(&db1).expect("graph 1");
        let g2 = load_graph_from_db(&db2).expect("graph 2");
        let expected = codetopo_cli::diff_value(&g1, &g2);
        assert_eq!(expected["added_nodes"][0]["id"], "pair::main::d");

        let via_tool = result_of(
            "codetopo_diff",
            &json!({
                "before_db_path": db1.display().to_string(),
                "after_db_path": db2.display().to_string(),
            }),
        );
        assert_eq!(via_tool, expected, "MCP diff (db pair) == shared diff");

        // The snapshot pair reports the identical diff.
        let snap1 = dir.path().join("one.json");
        let snap2 = dir.path().join("two.json");
        codetopo_cli::snapshot_db(&db1, &snap1, None).expect("snapshot 1");
        codetopo_cli::snapshot_db(&db2, &snap2, None).expect("snapshot 2");
        let via_snaps = result_of(
            "codetopo_diff",
            &json!({
                "before_snapshot": snap1.display().to_string(),
                "after_snapshot": snap2.display().to_string(),
            }),
        );
        assert_eq!(via_snaps, expected, "MCP diff (snapshot pair) == shared diff");

        // Exactly one source per side: neither and both are refused.
        assert!(call_tool("codetopo_diff", &json!({})).is_err());
        assert!(call_tool(
            "codetopo_diff",
            &json!({
                "before_db_path": db1.display().to_string(),
                "before_snapshot": snap1.display().to_string(),
                "after_db_path": db2.display().to_string(),
            })
        )
        .is_err());
    }

    #[test]
    fn get_symbol_returns_the_core_dossier_for_every_node() {
        let (_dir, db_path) = fixture_index();
        let graph = load_graph_from_db(&db_path).expect("graph loads");
        for node in graph.nodes() {
            let fetched =
                result_of("codetopo_get_symbol", &args(&db_path, json!({"node": node.id})));
            let core = serde_json::to_value(graph.dossier(&node.id).expect("dossier"))
                .expect("dossier serializes");
            assert_eq!(fetched, core, "dossier parity for {}", node.id);
        }
    }
}
