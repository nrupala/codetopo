// Copyright (C) 2026 2351485 Alberta Inc.
// SPDX-License-Identifier: AGPL-3.0-or-later
// Part of codetopo — Owned by 2351485 ALBERTA INC. (https://nrupalakolkar.com) · Built with Muse by Meta (https://muse.ai/join).

//! Known-answer tests for the composite `diagnose_hotspot` query.
//!
//! The fixture plants a hotspot by construction: `hub` is called by
//! three leaves and, transitively, by `top`, so its blast radius is
//! exactly 4 — larger than any other symbol's — and it makes one
//! dotted call (`external_service.fetch()`) that extraction cannot
//! resolve, so the graph records it against an `external::` stub and
//! the index carries an unresolved-call diagnostic. The composite
//! must find the planted hotspot first, report the planted counts,
//! and state the unresolved reference instead of hiding it.

use std::path::{Path, PathBuf};
use std::process::Command;

use codetopo_cli::{diagnose_hotspot_value, index_repo, load_diagnostics_from_db, load_graph_from_db};
use codetopo_core::RankMetric;

const HUB_PY: &str = "\
def hub():
    return external_service.fetch()


def leaf_a():
    return hub()


def leaf_b():
    return hub()


def leaf_c():
    return hub()


def top():
    leaf_a()
    leaf_b()
    leaf_c()
";

/// Write the fixture into a fresh scratch dir (the cli test suite's
/// workdir idiom) and index it. Returns (repo, db).
///
/// One scratch dir per CALL: this binary's tests run as parallel
/// threads of one process, so a PID-only directory name is shared
/// by all of them and each call's `remove_dir_all` deletes the
/// fixture from under its siblings — the intermittent CI failure
/// "skipping hub.py: read error: No such file or directory". A
/// per-call counter keeps the directories distinct.
fn planted_index() -> (PathBuf, PathBuf) {
    use std::sync::atomic::{AtomicU64, Ordering};
    static CALLS: AtomicU64 = AtomicU64::new(0);
    let n = CALLS.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!(
        "codetopo-cli-test-{}-diagnose-hotspot-{}",
        std::process::id(),
        n
    ));
    let _ = std::fs::remove_dir_all(&dir);
    let repo = dir.join("repo");
    std::fs::create_dir_all(&repo).expect("create repo dir");
    std::fs::write(
        repo.join("pyproject.toml"),
        "[project]\nname = \"hubtest\"\nversion = \"0.1.0\"\n",
    )
    .expect("write pyproject");
    std::fs::write(repo.join("__init__.py"), "").expect("write __init__");
    std::fs::write(repo.join("hub.py"), HUB_PY).expect("write hub.py");
    let db = dir.join("index.db");
    let report = index_repo(&repo, &db, "hubtest", None, None).expect("fixture indexes");
    assert!(report.files_indexed > 0, "fixture indexed no files");
    (repo, db)
}

fn diagnose(db: &Path) -> serde_json::Value {
    let graph = load_graph_from_db(db).expect("graph loads");
    let diagnostics = load_diagnostics_from_db(db).expect("diagnostics load");
    diagnose_hotspot_value(&graph, &diagnostics, RankMetric::BlastRadius, 20)
}

#[test]
fn composite_finds_the_planted_hotspot_with_its_atomic_counts() {
    let (_repo, db) = planted_index();
    let value = diagnose(&db);

    assert_eq!(value["metric"], "blast_radius");
    let hotspots = value["hotspots"].as_array().expect("hotspots list");
    assert!(!hotspots.is_empty(), "no hotspots reported: {value}");

    // The planted hotspot leads, with the planted counts: blast
    // radius 4 (leaf_a, leaf_b, leaf_c, top), and the unresolved
    // dotted call it makes is stated, not hidden.
    let first = &hotspots[0];
    assert!(
        first["id"].as_str().expect("id").ends_with("::hub"),
        "planted hotspot is not first: {first}"
    );
    assert_eq!(first["rank_count"], 4, "hub's rank count: {first}");
    assert_eq!(first["blast_radius_size"], 4, "hub's blast radius: {first}");
    assert_eq!(
        first["unresolved_references"]["outgoing"], 1,
        "hub's unresolved outgoing reference: {first}"
    );
    assert_eq!(
        first["unresolved_references"]["total"], 1,
        "hub's unresolved references: {first}"
    );
    assert!(
        first["neighborhood_size"].as_u64().expect("neighborhood") >= 5,
        "hub's neighborhood covers its callers, its callee stub, and itself: {first}"
    );

    // The index-level summary states the unresolved call.
    let summary = &value["diagnostics_summary"];
    assert!(
        summary["unresolved_calls"].as_u64().expect("unresolved") >= 1,
        "diagnostics summary hides the unresolved call: {summary}"
    );
    assert_eq!(
        summary["by_kind"]["unresolved_call"], summary["unresolved_calls"],
        "by-kind tally and unresolved count disagree: {summary}"
    );
}

#[test]
fn composite_agrees_with_the_atomic_queries_it_composes() {
    let (_repo, db) = planted_index();
    let graph = load_graph_from_db(&db).expect("graph loads");
    let value = diagnose(&db);

    // Every hotspot's blast-radius size equals the atomic query's
    // own answer for the same id — the composite adds no measurement.
    for hotspot in value["hotspots"].as_array().expect("hotspots list") {
        let id = hotspot["id"].as_str().expect("id");
        assert_eq!(
            hotspot["blast_radius_size"].as_u64().expect("size") as usize,
            graph.blast_radius(id).len(),
            "composite disagrees with the atomic blast_radius for {id}"
        );
        assert_eq!(
            hotspot["neighborhood_size"].as_u64().expect("size") as usize,
            graph.neighborhood(id, 1).nodes.len(),
            "composite disagrees with the atomic neighborhood for {id}"
        );
    }
}

#[test]
fn cli_diagnose_hotspot_prints_the_composite_counts_first() {
    let (_repo, db) = planted_index();
    let bin = env!("CARGO_BIN_EXE_codetopo");
    let out = Command::new(bin)
        .args([
            "diagnose-hotspot",
            "--db",
            db.to_str().expect("db path"),
            "--json",
        ])
        .output()
        .expect("run diagnose-hotspot");
    assert!(
        out.status.success(),
        "diagnose-hotspot failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let value: serde_json::Value =
        serde_json::from_slice(&out.stdout).expect("stdout is the composite JSON");
    assert_eq!(value["metric"], "blast_radius");
    assert!(
        value["hotspots"][0]["id"]
            .as_str()
            .expect("id")
            .ends_with("::hub"),
        "CLI composite does not lead with the planted hotspot: {value}"
    );

    // The human form leads with the counts too.
    let out = Command::new(bin)
        .args(["diagnose-hotspot", "--db", db.to_str().expect("db path")])
        .output()
        .expect("run diagnose-hotspot (human)");
    assert!(out.status.success());
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(
        text.contains("hotspots by blast_radius"),
        "human form does not lead with counts: {text}"
    );
    assert!(
        text.contains("unresolved references 1"),
        "human form hides the unresolved reference: {text}"
    );
}
