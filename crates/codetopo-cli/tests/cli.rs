// Copyright (C) 2026 Nrupal Akolkar
// SPDX-License-Identifier: AGPL-3.0-or-later
// Part of codetopo — Owned by Nrupal Akolkar (https://nrupalakolkar.com) · Built with Muse by Meta (https://muse.ai/join) — join Muse, then redeem code 101EEK in Settings within 48 hours of joining and we both earn tokens.

//! Integration test for the `codetopo` CLI (worker W5).
//!
//! Builds a tiny 3-file fixture repo (a.rs: `fn a` calls `b`; b.rs: `fn b`;
//! c.ts: a class), drives the library's `index_repo` directly, then asserts
//! the aggregate counts, a `descendants` query, the snapshot (+ proof) files,
//! and a restore round-trip into a second database. A second test drives the
//! built binary to check the exit-code-2 contract for unknown ids.

use std::path::{Path, PathBuf};
use std::process::Command;

use codetopo_cli::{index_repo, load_graph_from_db, restore_snapshot, verify_db, verify_proof_file};
use codetopo_core::{EdgeKind, NodeKind};

/// Fresh scratch dir per test: <temp>/codetopo-cli-test-<pid>-<name>/.
/// The per-test `name` keeps parallel tests from sharing (and deleting)
/// each other's directories.
fn workdir(name: &str) -> PathBuf {
    let dir =
        std::env::temp_dir().join(format!("codetopo-cli-test-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create workdir");
    dir
}

/// Fixture repo: a.rs (`fn a` → calls `b`), b.rs (`fn b`), c.ts (a class).
fn fixture_repo(workdir: &Path) -> PathBuf {
    let repo = workdir.join("repo");
    std::fs::create_dir_all(&repo).expect("create repo dir");
    std::fs::write(repo.join("a.rs"), "fn a() {\n    b();\n}\n").expect("write a.rs");
    std::fs::write(repo.join("b.rs"), "fn b() {}\n").expect("write b.rs");
    std::fs::write(repo.join("c.ts"), "class C {\n}\n").expect("write c.ts");
    repo
}

/// Find a symbol node id by (kind, label) — robust to the extractor's exact
/// id scheme (`{package}::{module}::{name}`).
fn find_id(graph: &codetopo_core::Graph, kind: NodeKind, label: &str) -> String {
    graph
        .nodes()
        .find(|n| n.kind == kind && n.label == label)
        .unwrap_or_else(|| panic!("expected a {kind:?} node labeled '{label}'"))
        .id
        .clone()
}

#[test]
fn index_query_snapshot_restore_round_trip() {
    let workdir = workdir("round-trip");
    let repo = fixture_repo(&workdir);
    let db = workdir.join("index.db");
    let snap = workdir.join("snap.json");
    let db2 = workdir.join("restored.db");

    // --- index (as a library call) ---
    let report = index_repo(&repo, &db, "fixture", Some(&snap), Some(b"test-key"))
        .expect("index_repo succeeds");
    assert_eq!(report.files_indexed, 3, "all three fixture files indexed");
    assert_eq!(report.files_failed, 0, "no fixture file may fail extraction");
    // 1 package + 3 modules + 3 files + 3 symbols (a, b, C) = 10 nodes;
    // 9 contains edges + 1 calls edge (a -> b) = 10 edges. Lower bounds: the
    // extractor may emit more, never less.
    assert!(report.nodes >= 10, "nodes >= 10, got {}", report.nodes);
    assert!(report.edges >= 10, "edges >= 10, got {}", report.edges);
    assert!(db.exists(), "database file created");

    // --- query: descendants("...::a") contains b's id ---
    let graph = load_graph_from_db(&db).expect("load graph from db");
    let a_id = find_id(&graph, NodeKind::Function, "a");
    let b_fn_id = find_id(&graph, NodeKind::Function, "b");
    let _c_id = find_id(&graph, NodeKind::Class, "C");
    // NOTE (frozen cross-crate semantics): W3 names b.rs's module
    // `fixture::b` and its function `fixture::b::b`. Core resolves the call
    // target `b` by `::b` suffix within the same-package tier, picking the
    // lexicographically first candidate — the *module* node `fixture::b`,
    // not the function. So "b's id" here is the id the calls edge actually
    // resolved to; the test asserts the descendants query is consistent
    // with the stored calls edge rather than hardcoding that resolution.
    // Whether resolve should prefer symbol nodes over module nodes is a
    // core/extract design question — flagged in the W5 report.
    let call_target = graph
        .edges()
        .find(|e| e.kind == EdgeKind::Calls && e.from == a_id)
        .map(|e| e.to.clone())
        .expect("a calls something");
    assert!(
        call_target.ends_with("::b") || call_target == "fixture::b",
        "call target should be b-related, got {call_target}"
    );
    let desc = graph.descendants(&a_id);
    assert!(
        desc.contains(&call_target),
        "descendants({a_id}) must contain b's resolved id {call_target}; got {desc:?}"
    );
    assert!(graph.get(&b_fn_id).is_some(), "function b is indexed");
    // ancestors is the reverse view.
    assert!(graph.ancestors(&call_target).contains(&a_id));

    // --- snapshot file (+ proof, since a key was given) ---
    assert!(snap.exists(), "snapshot file written");
    let snap_json = std::fs::read_to_string(&snap).expect("read snapshot");
    assert!(snap_json.contains("fixture"), "snapshot mentions the package");
    let proof_path = format!("{}.proof", snap.display());
    assert!(
        PathBuf::from(&proof_path).exists(),
        "proof file written alongside the snapshot"
    );
    let cert: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&proof_path).expect("read proof"))
            .expect("proof is a JSON certificate");
    assert_eq!(cert["certificate"], "codetopo-snapshot-proof");
    assert_eq!(cert["algorithm"], "HMAC-SHA256");
    assert_eq!(cert["snapshot_sha256"].as_str().unwrap().len(), 64);
    assert_eq!(cert["signature"].as_str().unwrap().len(), 64);
    // v2 certificate fields: serial, issuer, signer-key fingerprint,
    // and a validity window.
    assert_eq!(cert["version"], 2);
    assert_eq!(cert["issuer"], "codetopo");
    assert_eq!(cert["signer"], "codetopo-cli");
    assert_eq!(cert["signer_key_id"].as_str().unwrap().len(), 16);
    assert!(cert["serial"].as_str().unwrap().starts_with("CT-"));
    assert!(cert["expires_at"].as_u64().unwrap() > cert["issued_at"].as_u64().unwrap());
    verify_proof_file(&snap, Some(b"test-key"), false)
        .expect("verify-proof passes on a fresh snapshot");

    // --- restore into a second db preserves counts ---
    let stats = restore_snapshot(&snap, &db2, Some(b"test-key"), false).expect("restore succeeds");
    let g1 = load_graph_from_db(&db).expect("reload db1");
    let g2 = load_graph_from_db(&db2).expect("reload db2");
    assert_eq!(g2.node_count(), g1.node_count(), "restore preserves node count");
    assert_eq!(g2.edge_count(), g1.edge_count(), "restore preserves edge count");
    assert_eq!(stats.nodes, g1.node_count());
    assert_eq!(stats.edges, g1.edge_count());
    // Queries work on the restored graph too.
    let a2 = find_id(&g2, NodeKind::Function, "a");
    let target2 = g2
        .edges()
        .find(|e| e.kind == EdgeKind::Calls && e.from == a2)
        .map(|e| e.to.clone())
        .expect("restored graph keeps the calls edge");
    assert!(g2.descendants(&a2).contains(&target2));

    let _ = std::fs::remove_dir_all(&workdir);
}

#[test]
fn unknown_id_exits_2() {
    let workdir = workdir("exit-code");
    let repo = fixture_repo(&workdir);
    let db = workdir.join("index.db");
    index_repo(&repo, &db, "fixture", None, None).expect("index_repo succeeds");

    let bin = env!("CARGO_BIN_EXE_codetopo");
    let status = Command::new(bin)
        .args(["descendants", "no::such::id", "--db"])
        .arg(&db)
        .status()
        .expect("run codetopo binary");
    assert_eq!(
        status.code(),
        Some(2),
        "unknown id must exit with code 2"
    );

    let _ = std::fs::remove_dir_all(&workdir);
}

/// `verify` must FAIL (Err -> non-zero exit) when the stored audit chain
/// has been tampered with — printing "chain valid: false" and exiting 0
/// would let a broken chain pass every script that checks the exit code.
#[test]
fn verify_db_fails_on_tampered_chain() {
    use codetopo_store::{AuditLog, Store};

    let workdir = workdir("verify-tamper");
    let repo = fixture_repo(&workdir);
    let db = workdir.join("index.db");
    index_repo(&repo, &db, "fixture", None, None).expect("index");

    // Untouched chain verifies.
    verify_db(&db).expect("fresh chain verifies");

    // Tamper: rewrite one entry's payload without recomputing its hash.
    let mut store = Store::open(&db).expect("open store");
    let audit = store.load_audit().expect("load audit");
    let mut entries = audit.entries().to_vec();
    assert!(entries.len() > 1, "fixture index writes multiple entries");
    entries[1].payload = "tampered".to_string();
    store
        .save_audit(&AuditLog::from_entries(entries))
        .expect("save tampered audit");
    drop(store);

    assert!(
        verify_db(&db).is_err(),
        "verify_db must error on a tampered chain"
    );
}

/// Reindexing the same database with a different repo must REPLACE the
/// graph, not union into it — a stale symbol from the first repo has
/// no business surviving the second index.
#[test]
fn reindex_replaces_not_unions() {
    let workdir = workdir("reindex-replaces");
    let repo = fixture_repo(&workdir);
    let db = workdir.join("index.db");
    index_repo(&repo, &db, "fixture", None, None).expect("first index");
    let g1 = load_graph_from_db(&db).expect("load first graph");
    assert!(g1.nodes().any(|n| n.label == "a"));

    // Second repo: only b.rs (no a, no c.ts).
    let repo2 = workdir.join("repo2");
    std::fs::create_dir_all(&repo2).expect("create repo2");
    std::fs::write(repo2.join("b.rs"), "fn b() {}\n").expect("write b.rs");
    index_repo(&repo2, &db, "fixture", None, None).expect("second index");
    let g2 = load_graph_from_db(&db).expect("load second graph");
    assert!(
        !g2.nodes().any(|n| n.label == "a"),
        "stale symbol 'a' survived a reindex (union semantics)"
    );
    assert!(g2.nodes().any(|n| n.label == "b"));
}

/// Restore verifies before it writes: wrong key, a tampered declared
/// head, and a missing key are all refused; only the right key (or, for
/// the chain alone, the explicit unverified flag) restores.
#[test]
fn restore_verification_gates() {
    let workdir = workdir("restore-gates");
    let repo = fixture_repo(&workdir);
    let db = workdir.join("index.db");
    let snap = workdir.join("snap.json");
    index_repo(&repo, &db, "fixture", Some(&snap), Some(b"right-key")).expect("index");

    // Proof exists, no key, no flag -> refused.
    let db_a = workdir.join("a.db");
    assert!(restore_snapshot(&snap, &db_a, None, false).is_err());
    // Wrong key -> refused.
    let db_b = workdir.join("b.db");
    assert!(restore_snapshot(&snap, &db_b, Some(b"wrong-key"), false).is_err());
    // Right key -> restores.
    let db_c = workdir.join("c.db");
    assert!(restore_snapshot(&snap, &db_c, Some(b"right-key"), false).is_ok());

    // Tamper with the declared audit head: the chain gate must refuse
    // even on the unverified path (the chain needs no key to check).
    let text = std::fs::read_to_string(&snap).expect("read snapshot");
    let head = codetopo_core::Snapshot::from_json(&text).unwrap().audit_head;
    let tampered = text.replacen(
        &format!("\"audit_head\": \"{head}\""),
        "\"audit_head\": \"0000000000000000000000000000000000000000000000000000000000000000\"",
        1,
    );
    assert_ne!(tampered, text, "tamper actually changed the snapshot");
    let snap2 = workdir.join("snap-tampered.json");
    std::fs::write(&snap2, &tampered).expect("write tampered snapshot");
    // The tampered file has no matching proof next to it either; the
    // chain gate fires first, so both paths must refuse.
    let db_d = workdir.join("d.db");
    assert!(restore_snapshot(&snap2, &db_d, Some(b"right-key"), false).is_err());
    let db_e = workdir.join("e.db");
    assert!(restore_snapshot(&snap2, &db_e, None, true).is_err());
}

/// An expired v2 certificate no longer attests: both verify-proof and
/// restore must refuse it, naming the expiry.
#[test]
fn expired_certificate_is_refused() {
    let workdir = workdir("expired-cert");
    let repo = fixture_repo(&workdir);
    let db = workdir.join("index.db");
    let snap = workdir.join("snap.json");
    index_repo(&repo, &db, "fixture", Some(&snap), Some(b"right-key")).expect("index");

    // Re-issue the same certificate with a validity window in the past,
    // signed correctly over the v2 canonical string.
    let proof_path = format!("{}.proof", snap.display());
    let mut cert: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&proof_path).expect("read proof"))
            .expect("parse cert");
    let canonical = format!(
        "codetopo-proof-v2\n{}\n{}\n{}\n{}\n0\n1000",
        cert["snapshot_sha256"].as_str().unwrap(),
        cert["audit_head"].as_str().unwrap(),
        cert["nodes"].as_u64().unwrap(),
        cert["edges"].as_u64().unwrap()
    );
    cert["issued_at"] = serde_json::json!(0);
    cert["expires_at"] = serde_json::json!(1000);
    cert["signature"] =
        serde_json::json!(codetopo_store::hmac_proof(b"right-key", &canonical));
    std::fs::write(&proof_path, serde_json::to_string_pretty(&cert).unwrap())
        .expect("write expired cert");

    let err = verify_proof_file(&snap, Some(b"right-key"), false)
        .expect_err("verify-proof must refuse an expired certificate");
    assert!(err.to_string().contains("expired"), "error names expiry, got: {err}");
    let db2 = workdir.join("restored.db");
    let err = restore_snapshot(&snap, &db2, Some(b"right-key"), false)
        .expect_err("restore must refuse an expired certificate");
    assert!(err.to_string().contains("expired"), "error names expiry, got: {err}");

    let _ = std::fs::remove_dir_all(&workdir);
}

/// Schema gate follows CODE_SCHEMA §8's major rule: a different major
/// is rejected; same-major drift is tolerated.
#[test]
fn restore_schema_major_rule() {
    let workdir = workdir("schema-major");
    let repo = fixture_repo(&workdir);
    let db = workdir.join("index.db");
    let snap = workdir.join("snap.json");
    index_repo(&repo, &db, "fixture", Some(&snap), None).expect("index without proof");
    let text = std::fs::read_to_string(&snap).expect("read snapshot");

    let snap2 = workdir.join("snap-v2.json");
    std::fs::write(&snap2, text.replace("1.1", "2.0")).expect("write v2 snapshot");
    let db2 = workdir.join("v2.db");
    let err = restore_snapshot(&snap2, &db2, None, true)
        .expect_err("schema major 2 must be rejected");
    assert!(
        err.to_string().contains("schema_version"),
        "error names the schema version, got: {err}"
    );

    let snap19 = workdir.join("snap-v19.json");
    std::fs::write(&snap19, text.replace("1.1", "1.9")).expect("write v1.9 snapshot");
    let db19 = workdir.join("v19.db");
    restore_snapshot(&snap19, &db19, None, true)
        .expect("same-major snapshot restores on the explicit unverified path");

    let _ = std::fs::remove_dir_all(&workdir);
}

/// verify-proof must fail when the snapshot's embedded audit chain was
/// altered — the chain gate needs no key and fires before the proof.
#[test]
fn verify_proof_rejects_altered_audit_chain() {
    let workdir = workdir("verify-altered-chain");
    let repo = fixture_repo(&workdir);
    let db = workdir.join("index.db");
    let snap = workdir.join("snap.json");
    index_repo(&repo, &db, "fixture", Some(&snap), Some(b"right-key")).expect("index");
    let text = std::fs::read_to_string(&snap).expect("read snapshot");
    std::fs::write(&snap, text.replace("index_started", "index_hacked"))
        .expect("alter one audit entry's op");
    assert!(
        verify_proof_file(&snap, Some(b"right-key"), false).is_err(),
        "an altered audit chain must fail verification"
    );

    let _ = std::fs::remove_dir_all(&workdir);
}

/// W1's scenario, kept alongside the repo-swap variant above: rename
/// a function and its call site, reindex the same database, and the
/// stored graph must equal a fresh index exactly — no stale node.
#[test]
fn reindex_replaces_stale_graph() {
    let workdir = workdir("reindex-replace");
    let repo = fixture_repo(&workdir);
    let db = workdir.join("index.db");

    index_repo(&repo, &db, "fixture", None, None).expect("first index");
    let g1 = load_graph_from_db(&db).expect("load first graph");
    assert!(g1.nodes().any(|n| n.kind == NodeKind::Function && n.label == "b"));

    std::fs::write(repo.join("a.rs"), "fn a() {\n    bb();\n}\n").expect("rewrite a.rs");
    std::fs::write(repo.join("b.rs"), "fn bb() {}\n").expect("rewrite b.rs");
    let report = index_repo(&repo, &db, "fixture", None, None).expect("reindex");
    let g2 = load_graph_from_db(&db).expect("load reindexed graph");

    assert!(
        !g2.nodes().any(|n| n.kind == NodeKind::Function && n.label == "b"),
        "stale function node 'b' must not survive reindex"
    );
    assert!(g2.nodes().any(|n| n.kind == NodeKind::Function && n.label == "bb"));

    let fresh_db = workdir.join("fresh.db");
    let fresh = index_repo(&repo, &fresh_db, "fixture", None, None).expect("fresh index");
    assert_eq!(report.nodes, fresh.nodes, "reindex node count == fresh count");
    assert_eq!(report.edges, fresh.edges, "reindex edge count == fresh count");
    assert_eq!(g2.node_count(), fresh.nodes);

    let _ = std::fs::remove_dir_all(&workdir);
}


/// The discovery surface (PR C): search / symbol / neighborhood / schema
/// and `--json` output, driven through the real binary.
#[test]
fn discovery_commands_and_json_output() {
    let workdir = workdir("discovery");
    let repo = fixture_repo(&workdir);
    let db = workdir.join("index.db");
    index_repo(&repo, &db, "fixture", None, None).expect("index_repo succeeds");
    let graph = load_graph_from_db(&db).expect("load graph");
    let b_id = find_id(&graph, NodeKind::Function, "b");
    let a_id = find_id(&graph, NodeKind::Function, "a");
    let bin = env!("CARGO_BIN_EXE_codetopo");
    let run = |args: &[&str]| {
        Command::new(bin)
            .args(args)
            .arg("--db")
            .arg(&db)
            .output()
            .expect("run codetopo binary")
    };

    // search finds b without knowing its id
    let out = run(&["search", "b"]);
    assert!(out.status.success());
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains(&b_id), "search prints b's id: {text}");

    // symbol describes the node in words
    let out = run(&["symbol", &b_id]);
    assert!(out.status.success());
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("tier:"), "symbol prints a tier: {text}");

    // unknown symbol id exits 2, like every other id-based command
    let out = run(&["symbol", "no::such::id"]);
    assert_eq!(out.status.code(), Some(2), "unknown symbol must exit 2");

    // neighborhood of a includes a itself
    let out = run(&["neighborhood", &a_id]);
    assert!(out.status.success());
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains(&a_id), "neighborhood lists the center: {text}");

    // --json shapes match the HTTP surface (minus index_id)
    let out = run(&["descendants", &a_id, "--json"]);
    assert!(out.status.success());
    let value: serde_json::Value =
        serde_json::from_slice(&out.stdout).expect("descendants --json is JSON");
    assert_eq!(value["relation"], "descendants");
    assert_eq!(value["node"], a_id);
    assert!(value["total"].as_u64().is_some());

    let out = run(&["search", "b", "--json"]);
    let value: serde_json::Value =
        serde_json::from_slice(&out.stdout).expect("search --json is JSON");
    assert_eq!(value["query"], "b");
    assert!(value["total"].as_u64().expect("total") >= 1);

    let out = Command::new(bin).args(["schema", "--json"]).output().expect("run schema");
    assert!(out.status.success());
    let value: serde_json::Value =
        serde_json::from_slice(&out.stdout).expect("schema --json is JSON");
    assert_eq!(value["schema_version"], "1.1");
    assert!(value["node_kinds"].as_array().expect("kinds").iter().any(|k| k == "function"));

    let out = run(&["stats", "--json"]);
    let value: serde_json::Value =
        serde_json::from_slice(&out.stdout).expect("stats --json is JSON");
    assert!(value["nodes"]["total"].as_u64().expect("node total") >= 10);

    let _ = std::fs::remove_dir_all(&workdir);
}

/// Package/workspace discovery: files under a Cargo.toml
/// `[package]` or a package.json `name` are indexed under that
/// package with package-relative paths (the extract contract's
/// terms); the nearest manifest wins; files under no manifest keep
/// the caller's fallback package.
#[test]
fn index_discovers_packages_from_manifests() {
    let work = workdir("packages");
    let repo = work.join("repo");
    // A virtual workspace root: [workspace] only, no package.
    std::fs::create_dir_all(repo.join("crates/alpha/src")).expect("mkdir alpha");
    std::fs::create_dir_all(repo.join("crates/alpha/nested/src")).expect("mkdir nested");
    std::fs::create_dir_all(repo.join("web")).expect("mkdir web");
    std::fs::write(repo.join("Cargo.toml"), "[workspace]\nmembers = [\"crates/*\"]\n")
        .expect("write root manifest");
    std::fs::write(repo.join("loose.rs"), "pub fn loose() {}\n").expect("write loose.rs");
    std::fs::write(
        repo.join("crates/alpha/Cargo.toml"),
        "[package]\nname = \"alpha-pkg\"\nversion = \"0.1.0\"\n",
    )
    .expect("write alpha manifest");
    std::fs::write(repo.join("crates/alpha/src/lib.rs"), "pub fn alpha_fn() {}\n")
        .expect("write lib.rs");
    std::fs::write(
        repo.join("crates/alpha/nested/Cargo.toml"),
        "[package]\nname = \"nested-pkg\"\n",
    )
    .expect("write nested manifest");
    std::fs::write(repo.join("crates/alpha/nested/src/deep.rs"), "pub fn deep_fn() {}\n")
        .expect("write deep.rs");
    std::fs::write(repo.join("web/package.json"), "{\"name\": \"web-pkg\"}\n")
        .expect("write package.json");
    std::fs::write(repo.join("web/app.ts"), "export function webFn() {}\n").expect("write app.ts");

    let db = work.join("graph.db");
    let report = index_repo(&repo, &db, "fallback-pkg", None, None).expect("index workspace");
    assert_eq!(report.files_failed, 0, "no file may fail: {report:?}");
    let graph = load_graph_from_db(&db).expect("load graph");

    for package in ["alpha-pkg", "nested-pkg", "web-pkg", "fallback-pkg"] {
        find_id(&graph, NodeKind::Package, package);
    }
    // Ids begin with the owning package, and module paths are
    // package-relative (the extractor drops `src/` and treats
    // lib.rs as the package root). Under the pre-discovery
    // behaviour every one of these would have worn `fallback-pkg`
    // and repo-relative module paths instead.
    let alpha = find_id(&graph, NodeKind::Function, "alpha_fn");
    assert_eq!(alpha, "alpha-pkg::alpha_fn");
    let deep = find_id(&graph, NodeKind::Function, "deep_fn");
    assert_eq!(deep, "nested-pkg::deep::deep_fn");
    let web = find_id(&graph, NodeKind::Function, "webFn");
    assert_eq!(web, "web-pkg::app::webFn");
    let loose = find_id(&graph, NodeKind::Function, "loose");
    assert_eq!(loose, "fallback-pkg::loose::loose");
}

/// Ranking on the CLI fixture (a calls b): `b` is the only node with a
/// blast radius, of exactly 1 — and the shared ranking object states
/// the metric and its method beside the numbers.
#[test]
fn rank_reports_the_fixture_hub() {
    let dir = workdir("rank");
    let repo = fixture_repo(&dir);
    let db = dir.join("index.db");
    index_repo(&repo, &db, "fixture", None, None).expect("index fixture");
    let graph = load_graph_from_db(&db).expect("load graph");
    let b_id = find_id(&graph, NodeKind::Function, "b");
    let value = codetopo_cli::rank_value(&graph, codetopo_core::RankMetric::BlastRadius, 20);
    assert_eq!(value["metric"], "blast_radius");
    assert_eq!(value["total"], 1);
    assert_eq!(value["results"][0]["id"], b_id);
    assert_eq!(value["results"][0]["count"], 1);
    let _ = std::fs::remove_dir_all(&dir);
}

/// The change-impact pair: two repos differing by one known edit —
/// `d` appended, and `c` now calls it. The diff is exactly that edit's
/// footprint: `d` added (with its contains edge), `c` changed (its
/// refs now name `d`), the calls and reads edges the schema derives
/// from the new call site, and `d`'s blast radius moving 0 → 1.
/// The snapshot pair reports the identical diff.
#[test]
fn diff_reports_exactly_the_one_known_edit() {
    let dir = workdir("diff");
    let write_repo = |name: &str, body: &str| {
        let repo = dir.join(name);
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
    let db1 = dir.join("one.db");
    let db2 = dir.join("two.db");
    index_repo(&repo1, &db1, "pair", None, None).expect("index repo1");
    index_repo(&repo2, &db2, "pair", None, None).expect("index repo2");
    let g1 = load_graph_from_db(&db1).expect("graph 1");
    let g2 = load_graph_from_db(&db2).expect("graph 2");

    let value = codetopo_cli::diff_value(&g1, &g2);
    let ids = |list: &serde_json::Value| -> Vec<String> {
        list.as_array()
            .expect("array")
            .iter()
            .map(|n| n["id"].as_str().expect("id").to_string())
            .collect()
    };
    assert_eq!(ids(&value["added_nodes"]), vec!["pair::main::d".to_string()]);
    assert!(ids(&value["removed_nodes"]).is_empty());
    assert_eq!(ids(&value["changed_nodes"]), vec!["pair::main::c".to_string()]);
    let edge_triples: Vec<(&str, &str, &str)> = value["added_edges"]
        .as_array()
        .expect("edges")
        .iter()
        .map(|e| {
            (
                e["from"].as_str().expect("from"),
                e["to"].as_str().expect("to"),
                e["kind"].as_str().expect("kind"),
            )
        })
        .collect();
    assert_eq!(
        edge_triples,
        vec![
            ("pair::/main.rs", "pair::main::d", "contains"),
            ("pair::main::c", "pair::main::d", "calls"),
            ("pair::main::c", "pair::main::d", "reads"),
        ]
    );
    assert!(value["removed_edges"].as_array().expect("edges").is_empty());
    let impact: Vec<(&str, u64, u64, i64)> = value["impact"]
        .as_array()
        .expect("impact")
        .iter()
        .map(|i| {
            (
                i["id"].as_str().expect("id"),
                i["blast_radius_before"].as_u64().expect("before"),
                i["blast_radius_after"].as_u64().expect("after"),
                i["delta"].as_i64().expect("delta"),
            )
        })
        .collect();
    assert_eq!(
        impact,
        vec![("pair::main::c", 0, 0, 0), ("pair::main::d", 0, 1, 1)]
    );

    // The snapshot pair reports the identical diff.
    let snap1 = dir.join("one.json");
    let snap2 = dir.join("two.json");
    codetopo_cli::snapshot_db(&db1, &snap1, None).expect("snapshot 1");
    codetopo_cli::snapshot_db(&db2, &snap2, None).expect("snapshot 2");
    let s1 = codetopo_cli::read_snapshot_graph(&snap1).expect("read snapshot 1");
    let s2 = codetopo_cli::read_snapshot_graph(&snap2).expect("read snapshot 2");
    assert_eq!(codetopo_cli::diff_value(&s1, &s2), value);

    let _ = std::fs::remove_dir_all(&dir);
}

/// The planted diagnostics fixture, end to end through the CLI library:
/// a mutual call (a cycle), a duplicated definition, and an unresolved
/// call are recorded at index time, and the shared JSON shape behind
/// `diagnostics --json` / the HTTP route / the MCP tool carries exactly
/// those three, serialized from the stored records.
#[test]
fn planted_repo_diagnostics_are_recorded_and_shared() {
    let dir = workdir("planted-diagnostics");
    let repo = dir.join("planted");
    std::fs::create_dir_all(&repo).expect("create repo dir");
    std::fs::write(
        repo.join("cyc.rs"),
        "pub fn a() {\n    b();\n    missing_fn();\n}\n\npub fn b() {\n    a();\n}\n",
    )
    .expect("write cyc.rs");
    std::fs::write(repo.join("dup.rs"), "pub fn d() {}\n\npub fn d() {}\n").expect("write dup.rs");
    let db = dir.join("planted.db");
    let report = index_repo(&repo, &db, "planted", None, None).expect("index planted repo");
    assert_eq!(report.diagnostics, 3, "planted index report: {report:?}");

    let value = codetopo_cli::diagnostics_value(&db).expect("diagnostics value");
    assert_eq!(value["total"], 3);
    let kinds: Vec<&str> = value["diagnostics"]
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

    let _ = std::fs::remove_dir_all(&dir);
}
