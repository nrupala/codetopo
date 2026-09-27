// Copyright (C) 2026 Nrupal Akolkar
// SPDX-License-Identifier: AGPL-3.0-or-later
// Part of codetopo — Owned by Nrupal Akolkar · Built with Muse by Meta.

//! codetopo-cli library — the implementation behind the `codetopo` binary.
//!
//! Kept as a library so integration tests (and future embedders) can drive
//! the logic directly without shelling out. `main.rs` is a thin clap wrapper
//! over these functions.
//!
//! Output discipline follows CODE_SCHEMA §6: aggregate-first. Every command
//! prints counts/summaries; full id dumps require explicit `--expand`.

use std::path::{Path, PathBuf};

use codetopo_core::snapshot::SnapshotAuditEntry;
use codetopo_core::{EdgeKind, Graph, GraphBuilder, Node, NodeKind, PathStep, Snapshot};
use codetopo_extract::{extract_file, is_supported};
use codetopo_store::{hmac_proof, AuditLog, InsertStats, Store};
use rayon::prelude::*;
use serde_json::json;
use thiserror::Error;
use walkdir::WalkDir;

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// CLI-level errors. Exit-code mapping: [`CliError::UnknownId`] → 2 (the
/// queried id is not in the graph); everything else → 1.
#[derive(Debug, Error)]
pub enum CliError {
    #[error("unknown id '{0}': not present in the graph")]
    UnknownId(String),
    #[error("store error: {0}")]
    Store(#[from] codetopo_store::StoreError),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("{0}")]
    Usage(String),
}

impl CliError {
    pub fn exit_code(&self) -> i32 {
        match self {
            CliError::UnknownId(_) => 2,
            _ => 1,
        }
    }
}

// ---------------------------------------------------------------------------
// HMAC key resolution
// ---------------------------------------------------------------------------

/// Resolve the HMAC key for snapshot proof certificates: the `--hmac-key`
/// flag first, then the `CODETOPO_HMAC_KEY` environment variable, else `None`
/// (the snapshot is written without a proof, with a stderr note).
pub fn resolve_hmac_key(flag: Option<&str>) -> Option<Vec<u8>> {
    let from_flag = flag.filter(|k| !k.is_empty()).map(|k| k.as_bytes().to_vec());
    if from_flag.is_some() {
        return from_flag;
    }
    std::env::var("CODETOPO_HMAC_KEY")
        .ok()
        .filter(|k| !k.is_empty())
        .map(|k| k.into_bytes())
}

// ---------------------------------------------------------------------------
// index
// ---------------------------------------------------------------------------

/// Aggregate report returned by [`index_repo`] (and printed to stdout).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IndexReport {
    pub files_indexed: usize,
    pub files_failed: usize,
    pub nodes: usize,
    pub edges: usize,
    pub diagnostics: usize,
}

/// Directory names the repo walker never descends into: VCS/build/dependency
/// output plus hidden directories (and `.corpus`, the fixture convention).
fn skipped_dir_name(name: &str) -> bool {
    matches!(name, ".git" | "target" | "node_modules" | ".corpus") || name.starts_with('.')
}

/// Walk `<repo>`, extract supported files in parallel, build the graph,
/// persist it, record the audit entries, and optionally write a snapshot.
///
/// Prints the aggregate (CODE_SCHEMA §6 aggregate-first) and returns it.
/// Extraction failures are non-fatal: the file is counted as failed, a
/// warning goes to stderr, and indexing continues. Exit code is always 0 on
/// success — hard failures surface as [`CliError`] (exit 1).
pub fn index_repo(
    repo: &Path,
    db_path: &Path,
    package: &str,
    snapshot_path: Option<&Path>,
    hmac_key: Option<&[u8]>,
) -> Result<IndexReport, CliError> {
    if !repo.is_dir() {
        return Err(CliError::Usage(format!(
            "repo is not a directory: {}",
            repo.display()
        )));
    }

    // 1. Collect supported files (sorted for deterministic processing).
    let mut files: Vec<PathBuf> = Vec::new();
    for entry in WalkDir::new(repo).into_iter().filter_entry(|e| {
        if e.file_type().is_dir() {
            e.depth() == 0 || !skipped_dir_name(&e.file_name().to_string_lossy())
        } else {
            true
        }
    }) {
        match entry {
            Ok(e) => {
                if e.file_type().is_file() && is_supported(e.path()) {
                    files.push(e.into_path());
                }
            }
            Err(err) => eprintln!("warning: walk error: {err}"),
        }
    }
    files.sort();

    // 2. Read + extract in parallel. `par_iter` over a slice preserves order
    //    on collect, so downstream processing stays deterministic.
    let extracted: Vec<(PathBuf, Result<codetopo_core::FileSymbols, String>)> = files
        .par_iter()
        .map(|path| {
            let rel = path.strip_prefix(repo).unwrap_or(path).to_path_buf();
            let result = std::fs::read_to_string(path)
                .map_err(|e| format!("read error: {e}"))
                .and_then(|src| extract_file(package, &rel, &src).map_err(|e| e.to_string()));
            (rel, result)
        })
        .collect();

    // 3. Build the graph sequentially (the builder is not shared-mutable).
    let mut builder = GraphBuilder::new();
    let mut files_indexed = 0usize;
    let mut files_failed = 0usize;
    for (rel, result) in extracted {
        match result {
            Ok(fs) => {
                builder.add_file(fs);
                files_indexed += 1;
            }
            Err(msg) => {
                files_failed += 1;
                eprintln!("warning: skipping {}: {}", rel.display(), msg);
            }
        }
    }
    let (graph, diags) = builder.build();

    // 4. Persist.
    let mut store = Store::open(db_path)?;
    let _inserted = store.insert_graph(&graph)?;

    // 5. Audit: index_started then index_finished, then save the chain.
    let mut audit = AuditLog::new();
    audit.record(
        "index_started",
        &json!({"repo": repo.to_string_lossy(), "package": package}).to_string(),
    );
    audit.record(
        "index_finished",
        &json!({
            "files_indexed": files_indexed,
            "files_failed": files_failed,
            "nodes": graph.node_count(),
            "edges": graph.edge_count(),
            "diagnostics": diags.len(),
        })
        .to_string(),
    );
    store.save_audit(&audit)?;

    // 6. Optional snapshot (+ proof when a key is available).
    if let Some(sp) = snapshot_path {
        write_snapshot_file(&graph, &audit.snapshot_entries(), sp, hmac_key)?;
    }

    // 7. Aggregate-first output.
    eprintln!("files indexed: {files_indexed}");
    eprintln!("files failed: {files_failed}");
    eprintln!("symbols: {}", graph.node_count());
    eprintln!("nodes: {}", graph.node_count());
    eprintln!("edges: {}", graph.edge_count());
    eprintln!("diagnostics: {}", diags.len());

    Ok(IndexReport {
        files_indexed,
        files_failed,
        nodes: graph.node_count(),
        edges: graph.edge_count(),
        diagnostics: diags.len(),
    })
}

// ---------------------------------------------------------------------------
// snapshot / restore
// ---------------------------------------------------------------------------

/// Write `Snapshot::from_graph(graph, audit_log)` as pretty JSON to `out`.
/// When `hmac_key` is present, also write `<out>.proof` containing
/// `hmac_proof(key, snapshot_json)`; otherwise a stderr note records that the
/// snapshot ships without a proof.
pub fn write_snapshot_file(
    graph: &Graph,
    audit_log: &[SnapshotAuditEntry],
    out: &Path,
    hmac_key: Option<&[u8]>,
) -> Result<(), CliError> {
    let json = Snapshot::from_graph(graph, audit_log).to_json_pretty()?;
    std::fs::write(out, &json)?;
    match hmac_key {
        Some(key) => {
            let proof = hmac_proof(key, &json);
            std::fs::write(format!("{}.proof", out.display()), proof)?;
        }
        None => eprintln!("note: no HMAC key provided; snapshot written without proof"),
    }
    Ok(())
}

/// Export the graph stored in `db_path` to `out` (+ `.proof` when a key is
/// available). The snapshot binds the current audit-log head.
pub fn snapshot_db(db_path: &Path, out: &Path, hmac_key: Option<&[u8]>) -> Result<(), CliError> {
    let store = Store::open(db_path)?;
    let graph = store.load_graph()?;
    let audit = store.load_audit()?;
    write_snapshot_file(&graph, &audit.snapshot_entries(), out, hmac_key)?;
    eprintln!("snapshot written: {}", out.display());
    Ok(())
}

/// Restore a snapshot JSON file into a (fresh) store at `db_path`.
/// The snapshot's audit trail is replayed verbatim, so the restored database
/// binds the same audit head the snapshot was exported under.
/// Returns the insert stats; prints the round-trip counts.
pub fn restore_snapshot(
    snapshot_path: &Path,
    db_path: &Path,
) -> Result<InsertStats, CliError> {
    let data = std::fs::read_to_string(snapshot_path)?;
    let snap = Snapshot::from_json(&data)?;
    let graph = snap.to_graph();
    let mut store = Store::open(db_path)?;
    let stats = store.insert_graph(&graph)?;
    store.save_audit(&AuditLog::from_snapshot_entries(&snap.audit_log))?;
    println!("restored nodes: {}", stats.nodes);
    println!("restored edges: {}", stats.edges);
    Ok(stats)
}

// ---------------------------------------------------------------------------
// queries
// ---------------------------------------------------------------------------

/// Open the store at `db_path` and load its full graph.
pub fn load_graph_from_db(db_path: &Path) -> Result<Graph, CliError> {
    let store = Store::open(db_path)?;
    Ok(store.load_graph()?)
}

/// Fetch a node or fail with exit-code-2 [`CliError::UnknownId`].
pub fn require_node<'g>(graph: &'g Graph, id: &str) -> Result<&'g Node, CliError> {
    graph.get(id).ok_or_else(|| CliError::UnknownId(id.to_string()))
}

/// Aggregate-first id list: the total count, then the top-`limit` ids by
/// default or every id with `--expand`.
pub fn print_id_list(title: &str, ids: &[String], expand: bool, limit: usize) {
    let shown: &[String] = if expand { ids } else { &ids[..ids.len().min(limit)] };
    println!("{} {title}", ids.len());
    for id in shown {
        println!("{id}");
    }
}

/// Render typed path steps as `a --calls--> x --imports--> b`.
pub fn render_path(steps: &[PathStep]) -> String {
    let mut out = String::new();
    if let Some(first) = steps.first() {
        out.push_str(&first.from);
        for s in steps {
            out.push_str(&format!(" --{}--> {}", s.kind.as_str(), s.to));
        }
    }
    out
}

/// Wire name of a node kind (`"function"`, `"class"`, …).
///
/// Public so the binary surfaces (`codetopo-server`, `codetopo-mcp`) tally
/// kinds with the same names the CLI prints, rather than each carrying a copy
/// of this table.
pub fn node_kind_name(kind: NodeKind) -> &'static str {
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

/// Every node kind, in schema order — the tally order for kind breakdowns.
pub const ALL_NODE_KINDS: [NodeKind; 10] = [
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

/// Every edge kind, in schema order — the tally order for kind breakdowns.
pub const ALL_EDGE_KINDS: [EdgeKind; 9] = [
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

/// Aggregate-first store statistics: totals plus per-kind breakdowns
/// (non-zero kinds only).
pub fn print_stats(graph: &Graph) {
    println!("nodes: {}", graph.node_count());
    for kind in ALL_NODE_KINDS {
        let n = graph.nodes().filter(|nd| nd.kind == kind).count();
        if n > 0 {
            println!("  {}: {n}", node_kind_name(kind));
        }
    }
    println!("edges: {}", graph.edge_count());
    for kind in ALL_EDGE_KINDS {
        let n = graph.edge_count_by_kind(kind);
        if n > 0 {
            println!("  {}: {n}", kind.as_str());
        }
    }
}

/// Verify the tamper-evident audit chain: entry count, validity, head hash.
pub fn verify_db(db_path: &Path) -> Result<(), CliError> {
    let store = Store::open(db_path)?;
    let audit = store.load_audit()?;
    println!("audit entries: {}", audit.len());
    println!("chain valid: {}", audit.verify());
    println!("head: {}", audit.head());
    Ok(())
}
