// Copyright (C) 2026 Nrupal Akolkar
// SPDX-License-Identifier: AGPL-3.0-or-later
// Part of codetopo — Owned by Nrupal Akolkar · Built with Muse by Meta.

//! codetopo-store — SQLite persistence for codetopo (Phase 1, worker W4).
//!
//! Two halves:
//! - [`Store`]: a rusqlite-backed store for [`Graph`] snapshots (nodes,
//!   edges), free-text annotations on nodes, and the tamper-evident audit
//!   log. Schema is created on open; graph inserts are single-transaction
//!   `INSERT OR IGNORE` upserts.
//! - Audit primitive: [`AuditLog`] (hash-chained append-only log) plus
//!   [`hmac_proof`]/[`verify_proof`] (hex HMAC-SHA256 proof certificates).
//!   This is the trust-primitive port (axiomcode pattern): every mutation of
//!   the graph can be bound to the log head, and the log's integrity is
//!   independently verifiable.
//!
//! Owned by Nrupal Akolkar · Built with Muse by Meta.

use std::path::Path;

use codetopo_core::snapshot::SCHEMA_VERSION;
use codetopo_core::{Edge, Graph, Loc, Node, NodeKind, Snapshot, Tier};
use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};
use thiserror::Error;

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Every fallible operation in this crate returns this error type.
#[derive(Debug, Error)]
pub enum StoreError {
    /// SQLite failure (includes transaction failures).
    #[error("sqlite error: {0}")]
    Sql(#[from] rusqlite::Error),
    /// JSON (de)serialization failure for defines/refs/attrs payloads.
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
    /// Filesystem failure (creating parent directories for the DB path).
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

// ---------------------------------------------------------------------------
// Schema
// ---------------------------------------------------------------------------

/// DDL executed on every open (idempotent — `IF NOT EXISTS`). Nodes and
/// edges store enums as snake_case strings matching core's serde wire names,
/// and the list/map fields as JSON strings.
const SCHEMA_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS nodes(
    id TEXT PRIMARY KEY,
    kind TEXT NOT NULL,
    label TEXT NOT NULL,
    file TEXT NOT NULL,
    line INTEGER NOT NULL,
    col INTEGER,
    tier TEXT NOT NULL,
    defines_json TEXT NOT NULL,
    refs_json TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS edges(
    from_id TEXT NOT NULL,
    to_id TEXT NOT NULL,
    kind TEXT NOT NULL,
    line INTEGER,
    attrs_json TEXT NOT NULL,
    PRIMARY KEY(from_id, to_id, kind)
);
CREATE TABLE IF NOT EXISTS annotations(
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    target_id TEXT NOT NULL,
    body TEXT NOT NULL,
    ts TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS audit_log(
    seq INTEGER PRIMARY KEY,
    ts TEXT NOT NULL,
    op TEXT NOT NULL,
    payload TEXT NOT NULL,
    prev_hash TEXT NOT NULL,
    hash TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_edges_from ON edges(from_id);
CREATE INDEX IF NOT EXISTS idx_edges_to ON edges(to_id);
"#;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Seconds since the UNIX epoch, decimal string. std::time only — no chrono
/// dependency, per the crate contract.
fn now_ts() -> String {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs().to_string())
        .unwrap_or_else(|_| "0".to_string())
}

/// Canonical snake_case wire name for a [`NodeKind`] (matches core's serde).
fn node_kind_str(k: NodeKind) -> &'static str {
    match k {
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

/// Canonical snake_case wire name for a [`Tier`] (matches core's serde).
fn tier_str(t: Tier) -> &'static str {
    match t {
        Tier::Core => "core",
        Tier::Recommended => "recommended",
        Tier::Optional => "optional",
    }
}

/// Parse a stored snake_case string back into a core enum via its serde impl
/// (so the store can never drift from core's wire names).
fn parse_kind(s: &str) -> Result<NodeKind, serde_json::Error> {
    serde_json::from_value(serde_json::Value::String(s.to_string()))
}

fn parse_tier(s: &str) -> Result<Tier, serde_json::Error> {
    serde_json::from_value(serde_json::Value::String(s.to_string()))
}

/// JSON failure inside a row-mapping closure: rusqlite's `query_map`
/// demands `Result<T, rusqlite::Error>`, so the serde error is wrapped as a
/// conversion failure at the offending column.
fn conv_err(col: usize, e: serde_json::Error) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(
        col,
        rusqlite::types::Type::Text,
        Box::new(e),
    )
}

/// Audit-entry hash: `hex(SHA-256 over "{seq}|{ts}|{op}|{payload}|{prev_hash}")`.
///
/// The format is fixed and documented here on purpose: any verifier (in any
/// language) recomputes exactly this byte string — five fields joined by
/// literal `|` separators, hashed with SHA-256, rendered lowercase hex.
fn entry_hash(seq: u64, ts: &str, op: &str, payload: &str, prev_hash: &str) -> String {
    let canon = format!("{}|{}|{}|{}|{}", seq, ts, op, payload, prev_hash);
    let digest = Sha256::digest(canon.as_bytes());
    hex::encode(digest)
}

// ---------------------------------------------------------------------------
// Audit log (hash-chained, tamper-evident)
// ---------------------------------------------------------------------------

/// One link in the audit chain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditEntry {
    pub seq: u64,
    pub ts: String,
    pub op: String,
    pub payload: String,
    pub prev_hash: String,
    pub hash: String,
}

/// Append-only, hash-chained audit log. Each entry's `hash` binds the full
/// entry (`seq|ts|op|payload|prev_hash`), and `prev_hash` binds the previous
/// entry's hash — so altering any entry breaks every later link
/// ([`AuditLog::verify`] detects this). The genesis entry is seq 0, op
/// `"genesis"`, payload `"{}"`, prev_hash `"GENESIS"`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditLog {
    entries: Vec<AuditEntry>,
}

impl AuditLog {
    /// New log with a single genesis entry.
    pub fn new() -> Self {
        let ts = now_ts();
        let hash = entry_hash(0, &ts, "genesis", "{}", "GENESIS");
        AuditLog {
            entries: vec![AuditEntry {
                seq: 0,
                ts,
                op: "genesis".to_string(),
                payload: "{}".to_string(),
                prev_hash: "GENESIS".to_string(),
                hash,
            }],
        }
    }

    /// Rebuild from stored entries (used by [`Store::load_audit`]). Entries
    /// are taken as-is — [`AuditLog::verify`] is the check that they are
    /// intact.
    pub fn from_entries(entries: Vec<AuditEntry>) -> Self {
        AuditLog { entries }
    }

    /// Append an op. `payload` is a JSON string (caller-serialized). Returns
    /// the new head hash.
    pub fn record(&mut self, op: &str, payload: &str) -> &str {
        let seq = self.entries.len() as u64;
        let ts = now_ts();
        let prev_hash = self.head().to_string();
        let hash = entry_hash(seq, &ts, op, payload, &prev_hash);
        self.entries.push(AuditEntry {
            seq,
            ts,
            op: op.to_string(),
            payload: payload.to_string(),
            prev_hash,
            hash,
        });
        self.head()
    }

    /// Head hash of the chain (the latest entry's `hash`).
    pub fn head(&self) -> &str {
        &self.entries.last().expect("audit log always has a genesis entry").hash
    }

    /// Recompute every hash link. Returns false on any mismatch: a wrong
    /// recomputed hash, a non-sequential seq, or a prev_hash that does not
    /// match the previous entry's hash.
    pub fn verify(&self) -> bool {
        let mut prev: Option<&AuditEntry> = None;
        for (i, e) in self.entries.iter().enumerate() {
            if e.seq != i as u64 {
                return false;
            }
            let want = entry_hash(e.seq, &e.ts, &e.op, &e.payload, &e.prev_hash);
            if want != e.hash {
                return false;
            }
            match prev {
                None => {
                    if e.prev_hash != "GENESIS" {
                        return false;
                    }
                }
                Some(p) => {
                    if e.prev_hash != p.hash {
                        return false;
                    }
                }
            }
            prev = Some(e);
        }
        true
    }

    pub fn entries(&self) -> &[AuditEntry] {
        &self.entries
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

impl Default for AuditLog {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// HMAC proof certificates
// ---------------------------------------------------------------------------

/// hex(HMAC-SHA256(key, message)). The proof primitive behind the audit
/// certificates: binds an artifact (e.g. a snapshot JSON) to a signer key.
pub fn hmac_proof(key: &[u8], message: &str) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(key)
        .expect("HMAC accepts keys of any length");
    mac.update(message.as_bytes());
    hex::encode(mac.finalize().into_bytes())
}

/// Constant-time verification of a hex proof. Returns false when the hex is
/// malformed or the MAC does not match — never panics on attacker input.
pub fn verify_proof(key: &[u8], message: &str, hex_sig: &str) -> bool {
    let sig = match hex::decode(hex_sig) {
        Ok(b) => b,
        Err(_) => return false,
    };
    let mut mac = Hmac::<Sha256>::new_from_slice(key)
        .expect("HMAC accepts keys of any length");
    mac.update(message.as_bytes());
    let want = mac.finalize().into_bytes();
    if sig.len() != want.len() {
        return false;
    }
    // Accumulate differences with no early exit: comparison time does not
    // depend on where (or whether) the bytes differ.
    let mut diff: u8 = 0;
    for (a, b) in want.iter().zip(sig.iter()) {
        diff |= a ^ b;
    }
    diff == 0
}

// ---------------------------------------------------------------------------
// Store
// ---------------------------------------------------------------------------

/// How many rows a graph insert actually wrote (0 for already-present rows —
/// inserts are `INSERT OR IGNORE`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InsertStats {
    pub nodes: usize,
    pub edges: usize,
}

/// A free-text note pinned to a node id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Annotation {
    pub id: i64,
    pub target_id: String,
    pub body: String,
    pub ts: String,
}

/// SQLite-backed persistence for codetopo graphs, annotations, and the audit
/// log. The connection is private: all access goes through these methods.
pub struct Store {
    conn: rusqlite::Connection,
}

impl Store {
    /// Open (or create) a file-backed database, creating parent directories
    /// as needed, and initialize the schema if the tables are missing.
    pub fn open(path: &Path) -> Result<Self, StoreError> {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)?;
            }
        }
        let conn = rusqlite::Connection::open(path)?;
        conn.execute_batch(SCHEMA_SQL)?;
        Ok(Store { conn })
    }

    /// Open an ephemeral in-memory database (schema initialized). Useful for
    /// tests and throwaway analysis sessions.
    pub fn open_in_memory() -> Result<Self, StoreError> {
        let conn = rusqlite::Connection::open_in_memory()?;
        conn.execute_batch(SCHEMA_SQL)?;
        Ok(Store { conn })
    }

    /// Insert a graph in a single transaction. Nodes and edges are
    /// `INSERT OR IGNORE` — re-inserting an already-stored graph is a no-op
    /// and `InsertStats` reports only rows actually written.
    pub fn insert_graph(&mut self, graph: &Graph) -> Result<InsertStats, StoreError> {
        let tx = self.conn.transaction()?;
        let mut nodes = 0usize;
        let mut edges = 0usize;
        {
            let mut ns = tx.prepare(
                "INSERT OR IGNORE INTO nodes(id, kind, label, file, line, col, tier, defines_json, refs_json)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            )?;
            for n in graph.nodes() {
                nodes += ns.execute(rusqlite::params![
                    n.id,
                    node_kind_str(n.kind),
                    n.label,
                    n.loc.file,
                    n.loc.line as i64,
                    n.loc.column.map(|c| c as i64),
                    tier_str(n.tier),
                    serde_json::to_string(&n.defines)?,
                    serde_json::to_string(&n.refs)?,
                ])?;
            }
            let mut es = tx.prepare(
                "INSERT OR IGNORE INTO edges(from_id, to_id, kind, line, attrs_json)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
            )?;
            for e in graph.edges() {
                edges += es.execute(rusqlite::params![
                    e.from,
                    e.to,
                    e.kind.as_str(),
                    e.line.map(|l| l as i64),
                    serde_json::to_string(&e.attrs)?,
                ])?;
            }
        }
        tx.commit()?;
        Ok(InsertStats { nodes, edges })
    }

    /// Look up one node by id. `Ok(None)` when the id is not stored.
    pub fn get_node(&self, id: &str) -> Result<Option<Node>, StoreError> {
        let mut stmt = self.conn.prepare(
            "SELECT id, kind, label, file, line, col, tier, defines_json, refs_json
             FROM nodes WHERE id = ?1",
        )?;
        let mut rows = stmt.query([id])?;
        match rows.next()? {
            Some(row) => Ok(Some(node_from_row(row)?)),
            None => Ok(None),
        }
    }

    /// `(node_count, edge_count)`.
    pub fn counts(&self) -> Result<(usize, usize), StoreError> {
        let n: i64 = self.conn.query_row("SELECT COUNT(*) FROM nodes", [], |r| r.get(0))?;
        let e: i64 = self.conn.query_row("SELECT COUNT(*) FROM edges", [], |r| r.get(0))?;
        Ok((n as usize, e as usize))
    }

    /// Load the whole stored graph: SELECT all nodes and edges, wrap in a
    /// [`Snapshot`] (`schema_version` = core's current draft, empty
    /// `audit_head` — the store does not invent an audit binding here), and
    /// rebuild via `Snapshot::to_graph()`.
    pub fn load_graph(&self) -> Result<Graph, StoreError> {
        let mut ns = self.conn.prepare(
            "SELECT id, kind, label, file, line, col, tier, defines_json, refs_json
             FROM nodes ORDER BY id",
        )?;
        let nodes: Vec<Node> = ns
            .query_map([], node_from_row)?
            .collect::<Result<Vec<_>, _>>()?;
        let mut es = self.conn.prepare(
            "SELECT from_id, to_id, kind, line, attrs_json
             FROM edges ORDER BY from_id, to_id, kind",
        )?;
        let edges: Vec<Edge> = es
            .query_map([], edge_from_row)?
            .collect::<Result<Vec<_>, _>>()?;
        let snap = Snapshot {
            schema_version: SCHEMA_VERSION.to_string(),
            nodes,
            edges,
            audit_head: String::new(),
        };
        Ok(snap.to_graph())
    }

    /// Pin a free-text annotation to a node id. Returns the new row id.
    pub fn add_annotation(&mut self, target_id: &str, body: &str) -> Result<i64, StoreError> {
        self.conn.execute(
            "INSERT INTO annotations(target_id, body, ts) VALUES (?1, ?2, ?3)",
            rusqlite::params![target_id, body, now_ts()],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// All annotations for a node id, oldest first.
    pub fn list_annotations(&self, target_id: &str) -> Result<Vec<Annotation>, StoreError> {
        let mut stmt = self.conn.prepare(
            "SELECT id, target_id, body, ts FROM annotations
             WHERE target_id = ?1 ORDER BY id",
        )?;
        let rows = stmt.query_map([target_id], |row| {
            Ok(Annotation {
                id: row.get(0)?,
                target_id: row.get(1)?,
                body: row.get(2)?,
                ts: row.get(3)?,
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// Replace the `audit_log` table contents with `log`'s entries, in one
    /// transaction. The store holds the chain; [`AuditLog::verify`] is what
    /// vouches for it.
    pub fn save_audit(&mut self, log: &AuditLog) -> Result<(), StoreError> {
        let tx = self.conn.transaction()?;
        tx.execute("DELETE FROM audit_log", [])?;
        {
            let mut stmt = tx.prepare(
                "INSERT INTO audit_log(seq, ts, op, payload, prev_hash, hash)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            )?;
            for e in log.entries() {
                stmt.execute(rusqlite::params![
                    e.seq as i64,
                    e.ts,
                    e.op,
                    e.payload,
                    e.prev_hash,
                    e.hash,
                ])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Load the audit log ordered by seq. An empty table yields a fresh
    /// genesis [`AuditLog::new()`] — the caller should [`AuditLog::verify`]
    /// before trusting it.
    pub fn load_audit(&self) -> Result<AuditLog, StoreError> {
        let mut stmt = self.conn.prepare(
            "SELECT seq, ts, op, payload, prev_hash, hash FROM audit_log ORDER BY seq",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(AuditEntry {
                seq: row.get::<_, i64>(0)? as u64,
                ts: row.get(1)?,
                op: row.get(2)?,
                payload: row.get(3)?,
                prev_hash: row.get(4)?,
                hash: row.get(5)?,
            })
        })?;
        let entries: Vec<AuditEntry> = rows.collect::<Result<Vec<_>, _>>()?;
        if entries.is_empty() {
            Ok(AuditLog::new())
        } else {
            Ok(AuditLog::from_entries(entries))
        }
    }
}

fn node_from_row(row: &rusqlite::Row<'_>) -> Result<Node, rusqlite::Error> {
    let kind_s: String = row.get(1)?;
    let tier_s: String = row.get(6)?;
    let defines_s: String = row.get(7)?;
    let refs_s: String = row.get(8)?;
    Ok(Node {
        id: row.get(0)?,
        kind: parse_kind(&kind_s).map_err(|e| conv_err(1, e))?,
        label: row.get(2)?,
        loc: Loc {
            file: row.get(3)?,
            line: row.get::<_, i64>(4)? as u32,
            column: row.get::<_, Option<i64>>(5)?.map(|c| c as u32),
        },
        tier: parse_tier(&tier_s).map_err(|e| conv_err(6, e))?,
        defines: serde_json::from_str(&defines_s).map_err(|e| conv_err(7, e))?,
        refs: serde_json::from_str(&refs_s).map_err(|e| conv_err(8, e))?,
    })
}

fn edge_from_row(row: &rusqlite::Row<'_>) -> Result<Edge, rusqlite::Error> {
    let kind_s: String = row.get(2)?;
    let attrs_s: String = row.get(4)?;
    Ok(Edge {
        from: row.get(0)?,
        to: row.get(1)?,
        kind: serde_json::from_value(serde_json::Value::String(kind_s))
            .map_err(|e| conv_err(2, e))?,
        line: row.get::<_, Option<i64>>(3)?.map(|l| l as u32),
        attrs: serde_json::from_str(&attrs_s).map_err(|e| conv_err(4, e))?,
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use codetopo_core::{
        EdgeKind, FileSymbols, GraphBuilder, NodeKind, RawCall, RawImport, RawSymbol,
    };

    fn sym(id: &str, kind: NodeKind, line: u32) -> RawSymbol {
        RawSymbol {
            id: id.to_string(),
            kind,
            label: id.rsplit("::").next().unwrap_or(id).to_string(),
            line,
            column: None,
            defines: Vec::new(),
            refs: Vec::new(),
        }
    }

    /// Tiny chain: d calls b, b calls e. Structural package/module/file
    /// nodes are added by the builder automatically.
    fn chain_graph() -> Graph {
        let mut b = GraphBuilder::new();
        b.add_file(FileSymbols {
            package: "t".to_string(),
            file: "m.rs".to_string(),
            module_id: "t::m".to_string(),
            symbols: vec![
                sym("t::m::d", NodeKind::Function, 1),
                sym("t::m::b", NodeKind::Function, 10),
                sym("t::m::e", NodeKind::Function, 20),
            ],
            imports: Vec::new(),
            calls: vec![
                RawCall { from_id: "t::m::d".to_string(), to_name: "b".to_string(), line: 2 },
                RawCall { from_id: "t::m::b".to_string(), to_name: "e".to_string(), line: 11 },
            ],
        });
        let (g, diags) = b.build();
        assert!(diags.is_empty(), "test fixture must be clean: {:?}", diags);
        g
    }

    #[test]
    fn insert_graph_reports_counts_and_is_idempotent() {
        let mut store = Store::open_in_memory().unwrap();
        let g = chain_graph();
        let stats = store.insert_graph(&g).unwrap();
        assert_eq!(stats.nodes, g.node_count());
        assert_eq!(stats.edges, g.edge_count());
        assert_eq!(store.counts().unwrap(), (g.node_count(), g.edge_count()));

        // Re-insert: INSERT OR IGNORE makes it a no-op.
        let stats2 = store.insert_graph(&g).unwrap();
        assert_eq!(stats2, InsertStats { nodes: 0, edges: 0 });
        assert_eq!(store.counts().unwrap(), (g.node_count(), g.edge_count()));
    }

    #[test]
    fn get_node_round_trips_all_fields() {
        let mut store = Store::open_in_memory().unwrap();
        let g = chain_graph();
        store.insert_graph(&g).unwrap();

        let n = store.get_node("t::m::b").unwrap().expect("node stored");
        let want = g.get("t::m::b").unwrap();
        assert_eq!(n, *want);
        assert_eq!(n.id, "t::m::b");
        assert_eq!(n.kind, NodeKind::Function);
        assert_eq!(n.label, "b");
        assert_eq!(n.loc.file, "m.rs");
        assert_eq!(n.loc.line, 10);
        assert_eq!(n.loc.column, None);
        assert_eq!(n.tier, Tier::Optional);

        assert!(store.get_node("t::m::nope").unwrap().is_none());
    }

    #[test]
    fn load_graph_preserves_counts_and_query_results() {
        let mut store = Store::open_in_memory().unwrap();
        let g = chain_graph();
        store.insert_graph(&g).unwrap();

        let g2 = store.load_graph().unwrap();
        assert_eq!(g2.node_count(), g.node_count());
        assert_eq!(g2.edge_count(), g.edge_count());
        assert_eq!(g2.descendants("t::m::d"), g.descendants("t::m::d"));
        assert_eq!(g2.descendants("t::m::d"), vec!["t::m::b".to_string(), "t::m::e".to_string()]);
    }

    #[test]
    fn audit_chain_verifies_and_tamper_is_detected() {
        let mut log = AuditLog::new();
        assert!(log.verify(), "genesis-only log is valid");

        log.record("index", r#"{"files":1}"#);
        log.record("query", r#"{"id":"t::m::d"}"#);
        log.record("export", r#"{"format":"json"}"#);
        assert_eq!(log.len(), 4);
        assert!(!log.is_empty());
        assert!(log.verify(), "untampered log verifies");

        // Clone, tamper one entry's payload in the clone, verify fails.
        let entries: Vec<AuditEntry> = log
            .entries()
            .iter()
            .map(|e| AuditEntry {
                seq: e.seq,
                ts: e.ts.clone(),
                op: e.op.clone(),
                payload: if e.seq == 2 {
                    r#"{"id":"ATTACKER"}"#.to_string()
                } else {
                    e.payload.clone()
                },
                prev_hash: e.prev_hash.clone(),
                hash: e.hash.clone(),
            })
            .collect();
        let tampered = AuditLog::from_entries(entries);
        assert_eq!(tampered.len(), 4);
        assert!(!tampered.verify(), "tampered payload breaks the chain");

        // Head is the latest entry's hash.
        assert_eq!(log.head(), log.entries().last().unwrap().hash);
    }

    #[test]
    fn audit_round_trips_through_store() {
        let mut store = Store::open_in_memory().unwrap();
        let mut log = AuditLog::new();
        log.record("index", r#"{"files":1}"#);
        let head = log.head().to_string();
        store.save_audit(&log).unwrap();

        let loaded = store.load_audit().unwrap();
        assert_eq!(loaded, log);
        assert_eq!(loaded.head(), head);
        assert!(loaded.verify());

        // Empty table -> fresh genesis log.
        let mut store2 = Store::open_in_memory().unwrap();
        let empty = store2.load_audit().unwrap();
        assert_eq!(empty.len(), 1);
        assert_eq!(empty.entries()[0].op, "genesis");
        assert!(empty.verify());

        // save_audit replaces (does not append).
        store2.save_audit(&log).unwrap();
        store2.save_audit(&AuditLog::new()).unwrap();
        assert_eq!(store2.load_audit().unwrap().len(), 1);
    }

    #[test]
    fn hmac_proof_round_trip_and_rejections() {
        let key = b"audit-signer-key";
        let msg = r#"{"snapshot":"deadbeef"}"#;
        let sig = hmac_proof(key, msg);
        assert_eq!(sig.len(), 64, "hex sha256 is 64 chars");
        assert!(verify_proof(key, msg, &sig), "round trip verifies");

        assert!(!verify_proof(b"wrong-key", msg, &sig), "wrong key rejected");
        assert!(!verify_proof(key, r#"{"snapshot":"tampered"}"#, &sig), "wrong message rejected");
        assert!(!verify_proof(key, msg, "zz"), "bad hex rejected");
        assert!(!verify_proof(key, msg, "abc"), "odd-length hex rejected");
        let other = hmac_proof(b"other", msg);
        assert!(!verify_proof(key, msg, &other), "valid hex, wrong MAC rejected");
    }

    #[test]
    fn annotations_add_and_list() {
        let mut store = Store::open_in_memory().unwrap();
        let id1 = store.add_annotation("t::m::d", "entry point worth reviewing").unwrap();
        let id2 = store.add_annotation("t::m::d", "second note").unwrap();
        assert!(id2 > id1);

        let list = store.list_annotations("t::m::d").unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].id, id1);
        assert_eq!(list[0].target_id, "t::m::d");
        assert_eq!(list[0].body, "entry point worth reviewing");
        assert!(!list[0].ts.is_empty());
        assert_eq!(list[1].body, "second note");

        assert!(store.list_annotations("t::m::unknown").unwrap().is_empty());
    }

    #[test]
    fn open_creates_parent_dirs() {
        let dir = std::env::temp_dir()
            .join(format!("codetopo-store-test-{}", std::process::id()));
        let db = dir.join("nested").join("db").join("store.sqlite");
        let mut store = Store::open(&db).unwrap();
        let g = chain_graph();
        store.insert_graph(&g).unwrap();
        assert_eq!(store.counts().unwrap(), (g.node_count(), g.edge_count()));
        assert!(db.exists(), "db file created under nested parents");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn edge_attrs_survive_round_trip() {
        use codetopo_core::schema::Edge;
        use std::collections::HashMap;
        // Build through the builder with a raw import carrying bindings.
        let mut b = GraphBuilder::new();
        b.add_file(FileSymbols {
            package: "p".to_string(),
            file: "a.rs".to_string(),
            module_id: "p::a".to_string(),
            symbols: vec![sym("p::a::x", NodeKind::Function, 1)],
            imports: vec![RawImport {
                from_module_id: "p::a".to_string(),
                to_module_id: "serde".to_string(),
                bindings: vec!["Serialize".to_string()],
                line: 3,
            }],
            calls: Vec::new(),
        });
        let (g, diags) = b.build();
        assert!(diags.is_empty());

        let mut store = Store::open_in_memory().unwrap();
        store.insert_graph(&g).unwrap();
        let g2 = store.load_graph().unwrap();

        let want: HashMap<_, _> = g
            .edges()
            .map(|e: &Edge| ((e.from.clone(), e.to.clone(), e.kind), (e.line, e.attrs.clone())))
            .collect();
        for e in g2.edges() {
            let got = want
                .get(&(e.from.clone(), e.to.clone(), e.kind))
                .expect("edge preserved");
            assert_eq!((e.line, e.attrs.clone()), *got, "line+attrs round-trip");
        }
        let imp = g2
            .edges()
            .find(|e| e.kind == EdgeKind::Imports)
            .expect("imports edge stored");
        assert_eq!(
            imp.attrs.get("bindings"),
            Some(&serde_json::Value::Array(vec![serde_json::Value::String(
                "Serialize".to_string()
            )]))
        );
    }
}
