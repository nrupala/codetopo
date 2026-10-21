// Copyright (C) 2026 Nrupal Akolkar
// SPDX-License-Identifier: AGPL-3.0-or-later
// Part of codetopo — Owned by Nrupal Akolkar (https://nrupalakolkar.com) · Built with Muse by Meta (https://muse.ai/join) — join Muse, then redeem code 101EEK in Settings within 48 hours of joining and we both earn tokens.

//! codetopo-cli library — the implementation behind the `codetopo` binary.
//!
//! Kept as a library so integration tests (and future embedders) can drive
//! the logic directly without shelling out. `main.rs` is a thin clap wrapper
//! over these functions.
//!
//! Output discipline follows CODE_SCHEMA §6: aggregate-first. Every command
//! prints counts/summaries; full id dumps require explicit `--expand`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use codetopo_core::snapshot::SnapshotAuditEntry;
use codetopo_core::{EdgeKind, Graph, GraphBuilder, Node, NodeKind, PathStep, Snapshot};
use codetopo_extract::{extract_file, is_supported};
use codetopo_store::{hmac_proof, sha256_hex, verify_proof, AuditLog, InsertStats, Store};
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

/// The package name every surface defaults to for a repository:
/// the repository directory's file stem, or `"repo"` when the path
/// has none. The CLI, the HTTP server, and the MCP tools all call
/// this — before it existed, the same fixture indexed as
/// `fixture-repo::…` via the CLI and MCP but `unknown::…` via HTTP,
/// so ids (and every query keyed on them) disagreed by surface.
pub fn default_package_name(repo: &Path) -> String {
    repo.file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "repo".to_string())
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

/// One discovered package: the directory a manifest lives in
/// (relative to the repo root, empty for the root itself) and the
/// package name that manifest declares.
struct PackageRoot {
    dir: PathBuf,
    name: String,
}

/// The `name` from a Cargo.toml's `[package]` table, or `None` when
/// the manifest declares no package (a virtual workspace root) or
/// the name is not a plain quoted string. A line scan, deliberately:
/// the name is the only field the indexer needs, and pulling in a
/// TOML parser for one field would invert the dependency weight of
/// this crate.
fn cargo_package_name(text: &str) -> Option<String> {
    let mut in_package = false;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_package = line == "[package]";
            continue;
        }
        if in_package && line.starts_with("name") {
            let value = line.split_once('=')?.1.trim();
            let unquoted = value
                .strip_prefix('"')
                .and_then(|v| v.strip_suffix('"'))
                .or_else(|| value.strip_prefix('\'').and_then(|v| v.strip_suffix('\'')));
            if let Some(name) = unquoted {
                if !name.is_empty() {
                    return Some(name.to_string());
                }
            }
        }
    }
    None
}

/// The `name` from a package.json, or `None`. Unlike the Cargo
/// scan this parses for real — the file is JSON, and serde_json is
/// already a dependency of everything in this workspace.
fn package_json_name(text: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(text).ok()?;
    value.get("name")?.as_str().filter(|n| !n.is_empty()).map(str::to_string)
}

/// The `name` from a pyproject.toml's `[project]` table (PEP 621),
/// or the legacy `[tool.poetry]` table. Same line-scan discipline
/// as the Cargo reader, for the same reason.
fn pyproject_package_name(text: &str) -> Option<String> {
    for table in ["[project]", "[tool.poetry]"] {
        let mut in_table = false;
        for line in text.lines() {
            let line = line.trim();
            if line.starts_with('[') {
                in_table = line == table;
                continue;
            }
            if in_table && line.starts_with("name") {
                let value = line.split_once('=')?.1.trim();
                let unquoted = value
                    .strip_prefix('"')
                    .and_then(|v| v.strip_suffix('"'))
                    .or_else(|| value.strip_prefix('\'').and_then(|v| v.strip_suffix('\'')));
                if let Some(name) = unquoted {
                    if !name.is_empty() {
                        return Some(name.to_string());
                    }
                }
            }
        }
    }
    None
}

/// Discover the package roots of a tree: every directory holding a
/// Cargo.toml with a `[package]` name, a package.json with a
/// `name`, or a pyproject.toml with a `[project]`/`[tool.poetry]`
/// name. The extract contract has always said a file's path is
/// relative to *its package root* and its ids begin with *its
/// package's name* — before this, the indexer passed one
/// caller-chosen package and repo-relative paths for the whole
/// tree, so a workspace's ids named the workspace root's directory
/// (`corpus::crates::codetopo-core::src::…`) instead of the crate
/// (`codetopo-core::src::…`).
fn discover_packages(repo: &Path) -> Vec<PackageRoot> {
    let mut roots = Vec::new();
    for entry in WalkDir::new(repo).into_iter().filter_entry(|e| {
        if e.file_type().is_dir() {
            e.depth() == 0 || !skipped_dir_name(&e.file_name().to_string_lossy())
        } else {
            true
        }
    }) {
        let Ok(entry) = entry else { continue };
        if !entry.file_type().is_file() {
            continue;
        }
        let file_name = entry.file_name().to_string_lossy();
        let name = match file_name.as_ref() {
            "Cargo.toml" => std::fs::read_to_string(entry.path())
                .ok()
                .and_then(|text| cargo_package_name(&text)),
            "package.json" => std::fs::read_to_string(entry.path())
                .ok()
                .and_then(|text| package_json_name(&text)),
            "pyproject.toml" => std::fs::read_to_string(entry.path())
                .ok()
                .and_then(|text| pyproject_package_name(&text)),
            _ => None,
        };
        if let Some(name) = name {
            let dir = entry
                .path()
                .parent()
                .and_then(|parent| parent.strip_prefix(repo).ok())
                .map(Path::to_path_buf)
                .unwrap_or_default();
            roots.push(PackageRoot { dir, name });
        }
    }
    roots
}

/// Resolve a repo-relative file path to (package name, path
/// relative to the package root): the *nearest* manifest directory
/// wins; a file under no manifest keeps the caller's package and
/// its repo-relative path, exactly as before discovery existed.
fn package_for_file(
    roots: &HashMap<PathBuf, String>,
    rel: &Path,
    fallback: &str,
) -> (String, PathBuf) {
    let mut ancestor = rel.parent().map(Path::to_path_buf);
    while let Some(dir) = ancestor {
        if let Some(name) = roots.get(&dir) {
            let inner = rel.strip_prefix(&dir).unwrap_or(rel).to_path_buf();
            return (name.clone(), inner);
        }
        ancestor = dir.parent().map(Path::to_path_buf);
    }
    (fallback.to_string(), rel.to_path_buf())
}

/// Walk `<repo>`, extract supported files in parallel, build the graph,
/// persist it, record the audit entries, and optionally write a snapshot.
///
/// `package` is the fallback identity: files under a discovered
/// manifest (Cargo.toml `[package]` / package.json `name`) belong
/// to that package instead, with paths relative to its root.
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

    // 1b. Discover package roots (Cargo.toml / package.json /
    //     pyproject.toml). Files under a manifest belong to that
    //     manifest's package, with paths relative to its root — the
    //     extract contract's terms.
    let package_roots: HashMap<PathBuf, String> = discover_packages(repo)
        .into_iter()
        .map(|root| (root.dir, root.name))
        .collect();

    // 2. Read + extract in parallel. `par_iter` over a slice preserves order
    //    on collect, so downstream processing stays deterministic.
    let extracted: Vec<(PathBuf, Result<codetopo_core::FileSymbols, String>)> = files
        .par_iter()
        .map(|path| {
            let rel = path.strip_prefix(repo).unwrap_or(path).to_path_buf();
            let (file_package, rel_in_package) = package_for_file(&package_roots, &rel, package);
            let result = std::fs::read_to_string(path)
                .map_err(|e| format!("read error: {e}"))
                .and_then(|src| {
                    extract_file(&file_package, &rel_in_package, &src).map_err(|e| e.to_string())
                });
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
    let _inserted = store.replace_graph(&graph)?;
    store.save_diagnostics(&diags)?;

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

    // 7. Aggregate-first output. `symbols` counts actual symbol nodes
    //    (classes, interfaces, functions, methods, variables) — not the
    //    structural package/module/file/external nodes in `nodes`.
    let symbols = graph
        .nodes()
        .filter(|n| {
            matches!(
                n.kind,
                NodeKind::Class
                    | NodeKind::Interface
                    | NodeKind::Function
                    | NodeKind::Method
                    | NodeKind::Variable
            )
        })
        .count();
    eprintln!("files indexed: {files_indexed}");
    eprintln!("files failed: {files_failed}");
    eprintln!("symbols: {symbols}");
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

/// Validity window of a v2 proof certificate: one year from issuance.
/// Expiry forces periodic re-verification instead of trusting an
/// ancient attestation forever.
pub const CERT_VALIDITY_SECS: u64 = 365 * 24 * 60 * 60;

/// Seconds since the UNIX epoch (0 if the clock is somehow before it).
fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// A fingerprint identifying a signing key without revealing it:
/// the first 16 hex characters of the key's SHA-256.
fn signer_key_id(key: &[u8]) -> String {
    sha256_hex(key)[..16].to_string()
}

/// The canonical string a v1 proof certificate signs: the certificate
/// format version, the snapshot's content hash, its audit head, and
/// its size. Binding all four means a signature vouches for *this*
/// snapshot — not for whatever file happens to sit next to it.
fn proof_canonical(snapshot_sha256: &str, audit_head: &str, nodes: usize, edges: usize) -> String {
    format!("codetopo-proof-v1\n{snapshot_sha256}\n{audit_head}\n{nodes}\n{edges}")
}

/// The canonical string a v2 certificate signs: the v1 bindings plus
/// the validity window, so neither endpoint can be edited without
/// invalidating the signature. (The serial is derived from the digest
/// and the issuer/signer fields are checked exactly at verification,
/// so they are bound without appearing here.)
fn proof_canonical_v2(
    snapshot_sha256: &str,
    audit_head: &str,
    nodes: usize,
    edges: usize,
    issued_at: u64,
    expires_at: u64,
) -> String {
    format!(
        "codetopo-proof-v2\n{snapshot_sha256}\n{audit_head}\n{nodes}\n{edges}\n{issued_at}\n{expires_at}"
    )
}

/// Write `Snapshot::from_graph(graph, audit_log)` as pretty JSON to `out`.
/// When `hmac_key` is present, also write `<out>.proof`: a structured
/// proof certificate (JSON, format v2) binding the snapshot's SHA-256,
/// its audit head, and its node/edge counts under an HMAC-SHA256
/// signature, with a serial number, issuer and signer-key fingerprint,
/// and a one-year validity window — the DESIGN §0.5 certificate shape,
/// not a bare MAC of the file. Without a key, a stderr note records
/// that the snapshot ships without a proof.
pub fn write_snapshot_file(
    graph: &Graph,
    audit_log: &[SnapshotAuditEntry],
    out: &Path,
    hmac_key: Option<&[u8]>,
) -> Result<(), CliError> {
    let snap = Snapshot::from_graph(graph, audit_log);
    let json = snap.to_json_pretty()?;
    std::fs::write(out, &json)?;
    match hmac_key {
        Some(key) => {
            let digest = sha256_hex(json.as_bytes());
            let issued_at = now_unix();
            let expires_at = issued_at + CERT_VALIDITY_SECS;
            let canonical = proof_canonical_v2(
                &digest,
                &snap.audit_head,
                snap.nodes.len(),
                snap.edges.len(),
                issued_at,
                expires_at,
            );
            let cert = serde_json::json!({
                "certificate": "codetopo-snapshot-proof",
                "version": 2,
                "algorithm": "HMAC-SHA256",
                "issuer": "codetopo",
                "signer": "codetopo-cli",
                "signer_key_id": signer_key_id(key),
                "serial": format!("CT-{}", &digest[..16]),
                "snapshot_sha256": digest,
                "audit_head": snap.audit_head,
                "nodes": snap.nodes.len(),
                "edges": snap.edges.len(),
                "issued_at": issued_at,
                "expires_at": expires_at,
                "signature": hmac_proof(key, &canonical),
            });
            std::fs::write(
                format!("{}.proof", out.display()),
                serde_json::to_string_pretty(&cert)?,
            )?;
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

/// The proof-certificate gate shared by restore and verify-proof:
/// verifies `<snapshot>.proof` against the snapshot's exact bytes and
/// returns the verification note. Certificate v1 (digest/head/counts/
/// signature) and v2 (the same bindings, plus serial, issuer and
/// signer-key fingerprint, and a signed validity window — expiry is
/// enforced) are both accepted; v1 verifies as what it is, a
/// certificate without a serial or validity window.
fn verify_certificate(
    snapshot_path: &Path,
    data: &str,
    snap: &Snapshot,
    hmac_key: Option<&[u8]>,
    allow_unverified: bool,
) -> Result<String, CliError> {
    let proof_path = format!("{}.proof", snapshot_path.display());
    match std::fs::read_to_string(&proof_path) {
        Ok(cert_text) => {
            let cert: serde_json::Value = serde_json::from_str(&cert_text)?;
            let version = cert["version"].as_u64().unwrap_or(0);
            let well_formed = cert["certificate"] == "codetopo-snapshot-proof"
                && (version == 1 || version == 2)
                && cert["algorithm"] == "HMAC-SHA256";
            if !well_formed {
                return Err(CliError::Usage(
                    "proof file is not a codetopo proof certificate".to_string(),
                ));
            }
            match hmac_key {
                Some(key) => {
                    let digest = sha256_hex(data.as_bytes());
                    let mut binds = cert["snapshot_sha256"] == digest
                        && cert["audit_head"] == snap.audit_head
                        && cert["nodes"] == snap.nodes.len()
                        && cert["edges"] == snap.edges.len();
                    let signature = cert["signature"].as_str().unwrap_or("");
                    let (canonical, note) = if version == 2 {
                        binds &= cert["issuer"] == "codetopo"
                            && cert["signer"] == "codetopo-cli"
                            && cert["signer_key_id"] == signer_key_id(key)
                            && cert["serial"] == format!("CT-{}", &digest[..16]);
                        let issued_at = cert["issued_at"].as_u64().unwrap_or(0);
                        let expires_at = cert["expires_at"].as_u64().unwrap_or(0);
                        if now_unix() > expires_at {
                            return Err(CliError::Usage(format!(
                                "proof certificate {} has expired (validity window ended at unix {})",
                                cert["serial"].as_str().unwrap_or("?"),
                                expires_at
                            )));
                        }
                        (
                            proof_canonical_v2(
                                &digest,
                                &snap.audit_head,
                                snap.nodes.len(),
                                snap.edges.len(),
                                issued_at,
                                expires_at,
                            ),
                            format!(
                                "proof certificate {} verified",
                                cert["serial"].as_str().unwrap_or("?")
                            ),
                        )
                    } else {
                        (
                            proof_canonical(
                                &digest,
                                &snap.audit_head,
                                snap.nodes.len(),
                                snap.edges.len(),
                            ),
                            "proof certificate verified (v1 — no serial or validity window)"
                                .to_string(),
                        )
                    };
                    if !binds || !verify_proof(key, &canonical, signature) {
                        return Err(CliError::Usage(
                            "proof certificate does not verify for this snapshot (digest, head, counts, identity, or signature mismatch)"
                                .to_string(),
                        ));
                    }
                    Ok(note)
                }
                None if allow_unverified => {
                    eprintln!(
                        "warning: proof certificate present but no key provided — restoring on audit-chain verification alone"
                    );
                    Ok("audit chain only — proof certificate NOT verified (no key)".to_string())
                }
                None => Err(CliError::Usage(
                    "proof certificate present but no HMAC key provided (pass --hmac-key / CODETOPO_HMAC_KEY, or --allow-unverified)"
                        .to_string(),
                )),
            }
        }
        Err(_) if allow_unverified => {
            eprintln!(
                "warning: no proof certificate found — restoring on audit-chain verification alone"
            );
            Ok("audit chain only — NO proof certificate".to_string())
        }
        Err(_) => Err(CliError::Usage(
            "no proof certificate found next to the snapshot (pass --allow-unverified to restore on audit-chain verification alone)"
                .to_string(),
        )),
    }
}

/// The full snapshot verification gate, shared by restore (which writes
/// only after this passes) and verify-proof (which writes nothing):
/// the schema major must be one this build speaks (CODE_SCHEMA §8),
/// the snapshot's audit chain must recompute and end at the head the
/// snapshot declares, and the proof certificate is verified whenever
/// it exists. Returns the parsed snapshot and the verification note.
fn verify_snapshot(
    snapshot_path: &Path,
    hmac_key: Option<&[u8]>,
    allow_unverified: bool,
) -> Result<(Snapshot, String), CliError> {
    let data = std::fs::read_to_string(snapshot_path)?;
    let snap = Snapshot::from_json(&data)?;
    snap.check_schema_version().map_err(CliError::Usage)?;
    let audit = AuditLog::from_snapshot_entries(&snap.audit_log);
    if snap.audit_log.is_empty() {
        // Nothing to recompute: a snapshot with no audit trail can only
        // ever restore on the explicit unverified path.
        if !allow_unverified {
            return Err(CliError::Usage(
                "snapshot carries no audit trail - nothing to verify it against (pass --allow-unverified to restore it anyway)".to_string(),
            ));
        }
        eprintln!("warning: snapshot carries no audit trail - restoring unverified");
    } else if !audit.verify() || audit.head() != snap.audit_head {
        return Err(CliError::Usage(
            "snapshot audit chain does not verify against its declared head — the snapshot has been altered or corrupted"
                .to_string(),
        ));
    }
    let proof_note = verify_certificate(snapshot_path, &data, &snap, hmac_key, allow_unverified)?;
    Ok((snap, proof_note))
}

/// Verify a snapshot end to end (schema, audit chain, proof
/// certificate) and print the report — writing nothing. This is the
/// `verify-proof` command: the same gate restore applies, runnable
/// on its own.
pub fn verify_proof_file(
    snapshot_path: &Path,
    hmac_key: Option<&[u8]>,
    allow_unverified: bool,
) -> Result<(), CliError> {
    let (snap, proof_note) = verify_snapshot(snapshot_path, hmac_key, allow_unverified)?;
    println!("schema version: {} (supported)", snap.schema_version);
    if snap.audit_log.is_empty() {
        println!("audit chain: not carried by this snapshot");
    } else {
        println!(
            "audit chain: valid ({} entries, head {})",
            snap.audit_log.len(),
            snap.audit_head
        );
    }
    println!("verified: {proof_note}");
    println!("status: VALID");
    Ok(())
}

/// Restore a snapshot JSON file into the store at `db_path`.
///
/// Verification comes first (see [`verify_snapshot`]) and is not
/// optional about the parts that need no key. Restoring with no
/// certificate at all, or with a certificate but no key, requires
/// `allow_unverified` and is loud about it. Only a verified snapshot
/// is written, and it REPLACES the stored graph atomically rather
/// than unioning into it.
pub fn restore_snapshot(
    snapshot_path: &Path,
    db_path: &Path,
    hmac_key: Option<&[u8]>,
    allow_unverified: bool,
) -> Result<InsertStats, CliError> {
    let (snap, proof_note) = verify_snapshot(snapshot_path, hmac_key, allow_unverified)?;
    let audit = AuditLog::from_snapshot_entries(&snap.audit_log);
    let graph = snap.to_graph();
    let mut store = Store::open(db_path)?;
    let stats = store.replace_graph(&graph)?;
    store.save_audit(&audit)?;
    println!("restored nodes: {}", stats.nodes);
    println!("restored edges: {}", stats.edges);
    println!("verified: {proof_note}");
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
/// The schema description every surface can serve (CLI `schema`,
/// HTTP `GET /v1/schema`, MCP `codetopo_describe_schema`): the node
/// and edge kinds, the tier classification rule, and the query
/// vocabulary — generated from the same constants the tallies use,
/// so it cannot drift from the implementation it describes.
pub fn describe_schema() -> serde_json::Value {
    serde_json::json!({
        "schema_version": codetopo_core::snapshot::SCHEMA_VERSION,
        "node_kinds": ALL_NODE_KINDS.iter().map(|k| node_kind_name(*k)).collect::<Vec<_>>(),
        "edge_kinds": ALL_EDGE_KINDS.iter().map(|k| k.as_str()).collect::<Vec<_>>(),
        "tiers": {
            "core": "package, module, file (the structural skeleton)",
            "recommended": "class, interface (type declarations)",
            "optional": "everything else",
        },
        "queries": ["descendants", "ancestors", "blast_radius", "path",
                    "neighborhood", "search", "symbol", "stats", "verify",
                    "diagnostics"],
    })
}

/// JSON form of an id-list answer, field-for-field the HTTP closure
/// response minus `index_id` (the CLI has no index ids).
pub fn print_id_list_json(relation: &str, node: &str, ids: &[String], expand: bool, limit: usize) {
    let total = ids.len();
    let shown: &[String] = if expand { ids } else { &ids[..total.min(limit)] };
    println!(
        "{}",
        serde_json::json!({
            "relation": relation,
            "node": node,
            "total": total,
            "expand": expand,
            "limit": limit,
            "truncated": !expand && total > limit,
            "nodes": shown,
        })
    );
}

/// JSON form of the stats answer, matching the HTTP stats shape
/// (`{nodes: {total, by_kind}, edges: {total, by_kind}}`).
pub fn print_stats_json(graph: &Graph) {
    let mut node_kinds = serde_json::Map::new();
    for kind in ALL_NODE_KINDS {
        let count = graph.nodes().filter(|n| n.kind == kind).count();
        if count > 0 {
            node_kinds.insert(node_kind_name(kind).to_string(), count.into());
        }
    }
    let mut edge_kinds = serde_json::Map::new();
    for kind in ALL_EDGE_KINDS {
        let count = graph.edge_count_by_kind(kind);
        if count > 0 {
            edge_kinds.insert(kind.as_str().to_string(), count.into());
        }
    }
    println!(
        "{}",
        serde_json::json!({
            "nodes": {"total": graph.node_count(), "by_kind": node_kinds},
            "edges": {"total": graph.edge_count(), "by_kind": edge_kinds},
        })
    );
}

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

/// Print the diagnostics recorded when the index at `db_path` was
/// built: the aggregate count first, then one line per diagnostic.
/// Diagnostics are persisted with the index, so this works long
/// after the indexing run.
pub fn print_diagnostics(db_path: &Path) -> Result<(), CliError> {
    let store = Store::open(db_path)?;
    let diags = store.load_diagnostics()?;
    println!("diagnostics: {}", diags.len());
    for d in &diags {
        println!("{d}");
    }
    Ok(())
}

/// JSON form of the diagnostics answer: aggregate-first
/// `{total, diagnostics}`, the diagnostics serialized by the core type
/// itself. This is the one computation behind the CLI's
/// `diagnostics --json`, the HTTP diagnostics route (which adds
/// `index_id`), and the MCP `codetopo_diagnostics` tool, so the three
/// surfaces cannot disagree about what was recorded at index time.
pub fn diagnostics_value(db_path: &Path) -> Result<serde_json::Value, CliError> {
    let store = Store::open(db_path)?;
    let diags = store.load_diagnostics()?;
    Ok(json!({"total": diags.len(), "diagnostics": diags}))
}

/// Print [`diagnostics_value`] as pretty JSON.
pub fn print_diagnostics_json(db_path: &Path) -> Result<(), CliError> {
    println!("{}", serde_json::to_string_pretty(&diagnostics_value(db_path)?)?);
    Ok(())
}

/// Verify the tamper-evident audit chain: entry count, validity, head hash.
pub fn verify_db(db_path: &Path) -> Result<(), CliError> {
    let store = Store::open(db_path)?;
    let audit = store.load_audit()?;
    let valid = audit.verify();
    println!("audit entries: {}", audit.len());
    println!("chain valid: {valid}");
    println!("head: {}", audit.head());
    if valid {
        Ok(())
    } else {
        Err(CliError::Usage(
            "audit chain verification FAILED — the stored chain does not \
             recompute; the database has been altered or corrupted"
                .to_string(),
        ))
    }
}

// ---------------------------------------------------------------------------
// tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cargo_name_comes_from_the_package_table_only() {
        // A [workspace] manifest declares no package.
        assert_eq!(cargo_package_name("[workspace]\nmembers = []\n"), None);
        // The package name is found even with dependency tables after it.
        let text = "[package]\nname = \"alpha-pkg\"\nversion = \"1.0.0\"\n\n[dependencies]\nserde = \"1\"\n";
        assert_eq!(cargo_package_name(text).as_deref(), Some("alpha-pkg"));
        // A `name` under any other table is not the package name.
        let other = "[workspace.package]\nname = \"not-a-package\"\n";
        assert_eq!(cargo_package_name(other), None);
        // Single-quoted TOML strings work too.
        assert_eq!(
            cargo_package_name("[package]\nname = 'single'\n").as_deref(),
            Some("single")
        );
    }

    #[test]
    fn pyproject_package_name_reads_project_then_poetry() {
        assert_eq!(
            pyproject_package_name("[project]\nname = \"py-pkg\"\n").as_deref(),
            Some("py-pkg")
        );
        assert_eq!(
            pyproject_package_name("[tool.poetry]\nname = 'poetry-pkg'\n").as_deref(),
            Some("poetry-pkg")
        );
        // A name in an unrelated table is not a package name.
        assert_eq!(
            pyproject_package_name("[tool.black]\nname = \"nope\"\n"),
            None
        );
        assert_eq!(pyproject_package_name("[project]\ndescription = \"x\"\n"), None);
    }

    #[test]
    fn package_json_name_reads_the_name_field() {
        assert_eq!(
            package_json_name(r#"{"name": "web-pkg", "private": true}"#).as_deref(),
            Some("web-pkg")
        );
        assert_eq!(package_json_name(r#"{"private": true}"#), None);
        assert_eq!(package_json_name("not json"), None);
    }

    #[test]
    fn nearest_manifest_wins_and_fallback_covers_the_rest() {
        let mut roots = HashMap::new();
        roots.insert(PathBuf::from("crates/alpha"), "alpha-pkg".to_string());
        roots.insert(
            PathBuf::from("crates/alpha/nested"),
            "nested-pkg".to_string(),
        );
        // Direct member of the outer package: path is package-relative.
        let (pkg, rel) = package_for_file(&roots, Path::new("crates/alpha/src/lib.rs"), "fb");
        assert_eq!(pkg, "alpha-pkg");
        assert_eq!(rel, PathBuf::from("src/lib.rs"));
        // The nested manifest is nearer and wins.
        let (pkg, rel) =
            package_for_file(&roots, Path::new("crates/alpha/nested/src/deep.rs"), "fb");
        assert_eq!(pkg, "nested-pkg");
        assert_eq!(rel, PathBuf::from("src/deep.rs"));
        // No manifest ancestor: the caller's package, repo-relative path.
        let (pkg, rel) = package_for_file(&roots, Path::new("loose.rs"), "fb");
        assert_eq!(pkg, "fb");
        assert_eq!(rel, PathBuf::from("loose.rs"));
    }
}
